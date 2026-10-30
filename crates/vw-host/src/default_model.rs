//! Seeds the product's model choice in its own `VIBE_HOME` on first start.
//!
//! Depends on the filesystem and TOML parser. Called by [`crate::launch::ProductLaunch`] before the
//! other config defaults. Existing Large 4 entries gain the missing published prices, the
//! missing output-token cap, the missing thinking levels and the missing default level (High);
//! a window or price that still holds the value an earlier version seeded takes the published one.
//! Every other configured value and the user's model choice remain unchanged.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
use toml_edit::{DocumentMut, value};

// USD per million tokens, published at https://docs.mistral.ai/models/mistral-large-4-0.
const INPUT_PRICE: f64 = 0.70;
const CACHED_INPUT_PRICE: f64 = 0.07;
const OUTPUT_PRICE: f64 = 2.10;
// The context window the Mistral API reports for mistral-large-4 (`max_context_length`).
const WINDOW: i64 = 1_048_576;
// The values earlier versions seeded: an entry still holding one was never changed by its user,
// so it takes the published value (any other value is the user's and stays).
const OLD_WINDOW: i64 = 1_000_000;
const OLD_INPUT_PRICE: f64 = 0.68;
const OLD_OUTPUT_PRICE: f64 = 2.09;

// The completion limit the Mistral API enforces for mistral-large-4 (a larger `max_tokens` is
// rejected with HTTP 400 "This model supports at most 262144 completion tokens",
// while `GET /v1/models/mistral-large-4` reports `max_output_tokens: null`).
// Vibe 3.0.1 and 3.0.2 sent `max_tokens = context window - estimated input` when a model had no
// `max_output_tokens`, which for a 1,000,000 window is about 987k, so every turn failed.
// Vibe 3.0.3 sends no `max_tokens` to Mistral unless a request sets its own; the value stays as
// the model's real completion limit, which Vibe passes on as a model fact.
const MAX_OUTPUT_TOKENS: i64 = 262_144;
// The thinking levels Mistral's API takes for Large 4 (a turn at each runs); it has no Off.
const THINKING_LEVELS: [&str; 3] = ["low", "medium", "high"];
// The level a chat thinks at until it picks one: High, as Large 4 always ran before its levels were
// listed (with an unset level Vibe would run the lowest listed one, Low).
const DEFAULT_THINKING: &str = "high";

const FIRST_CONFIG: &str = "active_model = \"large-4\"\n\n\
[[models]]\n\
name = \"mistral-large-4\"\n\
provider = \"mistral\"\n\
alias = \"large-4\"\n\
display_name = \"Large 4\"\n\
supports_images = true\n\
max_context_length = 1048576\n\
max_output_tokens = 262144\n\
thinking_levels = [\"low\", \"medium\", \"high\"]\n\
thinking = \"high\"\n\
input_price = 0.7\n\
cached_input_price = 0.07\n\
output_price = 2.1\n";

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

/// The config text with the missing prices, output cap, thinking levels and default level added
/// to our exact Large 4 entry (`name = "mistral-large-4"`, `provider = "mistral"`,
/// `alias = "large-4"`), or `None` when
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
        // A value an earlier version seeded becomes the published one, keeping its comment.
        for (key, old, new) in [
            ("input_price", OLD_INPUT_PRICE, INPUT_PRICE),
            ("output_price", OLD_OUTPUT_PRICE, OUTPUT_PRICE),
        ] {
            if let Some(item) = model.get_mut(key)
                && item.as_float() == Some(old)
                && let Some(current) = item.as_value_mut()
            {
                let decor = current.decor().clone();
                *current = new.into();
                *current.decor_mut() = decor;
                changed = true;
            }
        }
        if let Some(item) = model.get_mut("max_context_length")
            && item.as_integer() == Some(OLD_WINDOW)
            && let Some(current) = item.as_value_mut()
        {
            let decor = current.decor().clone();
            *current = WINDOW.into();
            *current.decor_mut() = decor;
            changed = true;
        }
        if !model.contains_key("max_output_tokens") {
            model["max_output_tokens"] = value(MAX_OUTPUT_TOKENS);
            changed = true;
        }
        // Large 4 always reasons and the key proxy sends Low, Medium and High as picked (Off
        // would run as High).
        if !model.contains_key("thinking_levels") {
            model["thinking_levels"] = value(
                THINKING_LEVELS
                    .iter()
                    .copied()
                    .collect::<toml_edit::Array>(),
            );
            changed = true;
        }
        // The default level only where the entry's levels offer it (a person's own levels
        // without High keep Vibe's choice among them).
        let offers_default = model
            .get("thinking_levels")
            .and_then(|item| item.as_array())
            .is_none_or(|levels| {
                levels
                    .iter()
                    .any(|level| level.as_str() == Some(DEFAULT_THINKING))
            });
        if !model.contains_key("thinking") && offers_default {
            model["thinking"] = value(DEFAULT_THINKING);
            changed = true;
        }
    }
    changed.then(|| config.to_string())
}
