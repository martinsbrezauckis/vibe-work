//! Seeds the product's model choice in its own `VIBE_HOME` on first start.
//!
//! Depends on the filesystem and TOML parser. Called by [`crate::launch::ProductLaunch`] before the
//! other config defaults. Existing Large 4 entries gain missing published prices; explicitly
//! configured prices and the user's model choice remain unchanged.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
use toml_edit::{DocumentMut, value};

// USD per million tokens, published at https://docs.mistral.ai/models/mistral-large-4-0
// (sale rates).
const INPUT_PRICE: f64 = 0.68;
const CACHED_INPUT_PRICE: f64 = 0.07;
const OUTPUT_PRICE: f64 = 2.09;

const FIRST_CONFIG: &str = "active_model = \"large-4\"\n\n\
[[models]]\n\
name = \"mistral-large-4\"\n\
provider = \"mistral\"\n\
alias = \"large-4\"\n\
display_name = \"Large 4\"\n\
supports_images = true\n\
max_context_length = 1000000\n\
input_price = 0.68\n\
cached_input_price = 0.07\n\
output_price = 2.09\n";

/// Writes the initial model config, or fills missing Large 4 prices in an existing one.
///
/// Returns `true` when this call created the file, or `false` when one already exists.
/// The caller creates `vibe_home` first. User-specified prices are preserved.
///
/// # Errors
/// Returns the filesystem error when the file cannot be created or written.
pub fn ensure(vibe_home: &Path) -> io::Result<bool> {
    let path = vibe_home.join("config.toml");
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fill_missing_prices(&path)?;
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    file.write_all(FIRST_CONFIG.as_bytes())?;
    Ok(true)
}

fn fill_missing_prices(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 * 1024 {
        return Ok(());
    }
    let original = std::fs::read_to_string(path)?;
    let Some(updated) = with_missing_prices(&original) else {
        return Ok(());
    };
    // A settings writer may have changed the file while this launch prepared it.
    if std::fs::read_to_string(path)? != original {
        return Ok(());
    }
    vw_platform::replace_private(path, updated.as_bytes()).map_err(io::Error::other)
}

fn with_missing_prices(original: &str) -> Option<String> {
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
    }
    changed.then(|| config.to_string())
}
