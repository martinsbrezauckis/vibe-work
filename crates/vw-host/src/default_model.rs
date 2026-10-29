//! Seeds the product's model choice in its own `VIBE_HOME` on first start.
//!
//! Depends on the filesystem and TOML parser. Called by [`crate::launch::ProductLaunch`] before the
//! other config defaults. Existing Large 4 entries gain the missing published prices and the
//! missing output-token cap; explicitly configured values and the user's model choice remain
//! unchanged.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
use toml_edit::{DocumentMut, value};

// USD per million tokens, published at https://docs.mistral.ai/models/mistral-large-4-0
// (sale rates).
const INPUT_PRICE: f64 = 0.68;
const CACHED_INPUT_PRICE: f64 = 0.07;
const OUTPUT_PRICE: f64 = 2.09;

// The completion limit the Mistral API enforces for mistral-large-4 (a larger `max_tokens` is
// rejected with HTTP 400 "This model supports at most 262144 completion tokens",
// while `GET /v1/models/mistral-large-4` reports `max_output_tokens: null`).
// Vibe 3.0.1 and 3.0.2 sent `max_tokens = context window - estimated input` when a model had no
// `max_output_tokens`, which for the 1,000,000 window is about 987k, so every turn failed.
// Vibe 3.0.3 sends no `max_tokens` to Mistral unless a request sets its own; the value stays as
// the model's real completion limit, which Vibe passes on as a model fact.
const MAX_OUTPUT_TOKENS: i64 = 262_144;

const FIRST_CONFIG: &str = "active_model = \"large-4\"\n\n\
[[models]]\n\
name = \"mistral-large-4\"\n\
provider = \"mistral\"\n\
alias = \"large-4\"\n\
display_name = \"Large 4\"\n\
supports_images = true\n\
max_context_length = 1000000\n\
max_output_tokens = 262144\n\
input_price = 0.68\n\
cached_input_price = 0.07\n\
output_price = 2.09\n";

/// Writes the initial model config, or fills missing Large 4 defaults in an existing one.
///
/// Returns `true` when this call created the file, or `false` when one already exists.
/// The caller creates `vibe_home` first. Values the user set (prices, output cap) are preserved.
///
/// # Errors
/// Returns the filesystem error when the file cannot be created or written.
pub fn ensure(vibe_home: &Path) -> io::Result<bool> {
    let path = vibe_home.join("config.toml");
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fill_missing_defaults(&path)?;
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    file.write_all(FIRST_CONFIG.as_bytes())?;
    Ok(true)
}

/// Rewrites `path` with [`with_missing_defaults`] when that changes it. Leaves alone a symlink,
/// a non-file, a file over 1 MiB, unparsable TOML, and a file that changed while it was read.
fn fill_missing_defaults(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 * 1024 {
        return Ok(());
    }
    let original = std::fs::read_to_string(path)?;
    let Some(updated) = with_missing_defaults(&original) else {
        return Ok(());
    };
    // A settings writer may have changed the file while this launch prepared it.
    if std::fs::read_to_string(path)? != original {
        return Ok(());
    }
    vw_platform::replace_private(path, updated.as_bytes()).map_err(io::Error::other)
}

/// The config text with the missing prices and output cap added to our exact Large 4 entry
/// (`name = "mistral-large-4"`, `provider = "mistral"`, `alias = "large-4"`), or `None` when
/// nothing is missing or the text is not a config with a `[[models]]` array. Keys already
/// present keep their value, whatever it is; other model entries are not touched.
fn with_missing_defaults(original: &str) -> Option<String> {
    let mut config: DocumentMut = original.parse().ok()?;
    let models = config.get_mut("models")?.as_array_of_tables_mut()?;
    let mut changed = false;
    for model in models.iter_mut() {
        if model.get("name").and_then(|item| item.as_str()) != Some("mistral-large-4")
            || model.get("provider").and_then(|item| item.as_str()) != Some("mistral")
            || model.get("alias").and_then(|item| item.as_str()) != Some("large-4")
        {
            continue;
        }
        for (key, price) in [
            ("input_price", INPUT_PRICE),
            ("cached_input_price", CACHED_INPUT_PRICE),
            ("output_price", OUTPUT_PRICE),
        ] {
            if !model.contains_key(key) {
                model[key] = value(price);
                changed = true;
            }
        }
        if !model.contains_key("max_output_tokens") {
            model["max_output_tokens"] = value(MAX_OUTPUT_TOKENS);
            changed = true;
        }
    }
    changed.then(|| config.to_string())
}
