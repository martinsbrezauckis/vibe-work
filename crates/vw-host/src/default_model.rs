//! Keeps an offline Large 4 fallback only while the provider's catalog is disabled.
//!
//! Depends on the TOML parser; written by [`crate::startup_config`] after catalog policy is staged. Product launch
//! applies the offline fallback; the first catalog-on launch in either mode removes an exact app seed. A failed
//! product write stops launch; development logs it and continues. With the catalog off,
//! existing Large 4 entries gain missing prices, output cap, thinking levels and compaction point; old seeded values
//! take their published updates. With the catalog on, an exact app seed is removed so Vibe uses the provider's model
//! list on the first catalog migration. A customized model row and unrelated settings remain unchanged.

use std::io;
use std::path::Path;
use toml_edit::{DocumentMut, value};

// USD per million tokens, published at https://docs.mistral.ai/models/mistral-large-4-0.
const INPUT_PRICE: f64 = 0.70;
const CACHED_INPUT_PRICE: f64 = 0.07;
const OUTPUT_PRICE: f64 = 2.10;
// The context window the Mistral API reports for mistral-large-4 (`max_context_length`).
const WINDOW: i64 = 1_048_576;
// Recognized seed values take the current published value; a different value stays unchanged.
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
// Where a chat on Large 4 compacts: the point Mistral's own Large 4 entry uses for this window (Vibe's
// default for an entry without one is 256,000). A person's own value is kept, and so is a person's own
// global point (top-level `auto_compact_threshold`): our entry then gets none.
const AUTO_COMPACT_THRESHOLD: i64 = 800_000;

const FIRST_CONFIG: &str = "active_model = \"large-4\"\n\n\
[[models]]\n\
name = \"mistral-large-4\"\n\
provider = \"mistral\"\n\
alias = \"large-4\"\n\
display_name = \"Mistral Large 4\"\n\
supports_images = true\n\
max_context_length = 1048576\n\
max_output_tokens = 262144\n\
thinking_levels = [\"low\", \"medium\", \"high\"]\n\
thinking = \"high\"\n\
input_price = 0.7\n\
cached_input_price = 0.07\n\
output_price = 2.1\n\
auto_compact_threshold = 800000\n";

/// What the Large 4 step did to the config text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelDefaults {
    /// There was no file: the first config was seeded.
    Created,
    /// Missing Large 4 defaults were filled.
    Filled,
    /// Nothing to change (also a file that is not a config with a `[[models]]` array).
    Unchanged,
}

/// Writes the initial model config, or fills missing Large 4 defaults in an existing one, through the guarded
/// startup write ([`crate::startup_config`]): a link, a non-file, a file over 1 MiB, unparsable TOML and a file that
/// changed while it was read are left alone.
///
/// Returns `true` when this call created the file, or `false` when one already exists.
/// Values the user set (prices, output cap) are preserved.
///
/// # Errors
/// Returns the filesystem error when the file cannot be read, created or written.
pub fn ensure(vibe_home: &Path) -> io::Result<bool> {
    let mut draft = crate::startup_config::Draft::open(vibe_home)?;
    let staged = draft.apply(edit, |_| ModelDefaults::Unchanged);
    let written = draft.commit();
    let outcome = written
        .settle(staged, |_| ModelDefaults::Unchanged)
        .map_err(io::Error::other)?;
    Ok(outcome == ModelDefaults::Created && written == crate::startup_config::Written::Saved)
}

/// This step's edit for [`crate::startup_config`]: the first config when `missing` (no file, no text yet), else
/// [`with_missing_defaults`].
pub(crate) fn edit(existing: &str, missing: bool) -> (Option<String>, ModelDefaults) {
    if existing
        .parse::<toml::Table>()
        .ok()
        .and_then(|config| config.get("experimental_enable_model_catalog")?.as_bool())
        == Some(true)
    {
        return match without_app_seed(existing) {
            Some(text) => (Some(text), ModelDefaults::Filled),
            None => (None, ModelDefaults::Unchanged),
        };
    }
    if missing {
        return (Some(FIRST_CONFIG.to_owned()), ModelDefaults::Created);
    }
    match with_missing_defaults(existing) {
        Some(text) => (Some(text), ModelDefaults::Filled),
        None => (None, ModelDefaults::Unchanged),
    }
}

/// Removes only the exact local row previously seeded by this app. A customized row and
/// the person's active-model choice stay for the catalog and Core to resolve.
fn without_app_seed(original: &str) -> Option<String> {
    // Compare after normalizing only recognized seed values; a different value stays different.
    let comparable = with_missing_defaults(original).unwrap_or_else(|| original.to_owned());
    let parsed: toml::Table = comparable.parse().ok()?;
    let seeded: toml::Table = FIRST_CONFIG.parse().ok()?;
    let seed = seeded.get("models")?.as_array()?.first()?;
    let index = parsed
        .get("models")?
        .as_array()?
        .iter()
        .position(|row| row == seed)?;
    let mut config: DocumentMut = original.parse().ok()?;
    let models = config.get_mut("models")?.as_array_of_tables_mut()?;
    models.remove(index);
    if models.is_empty() {
        config.remove("models");
    }
    Some(config.to_string())
}

/// The config text with the missing prices, output cap, thinking levels, default level and
/// compaction point added
/// to our exact Large 4 entry (`name = "mistral-large-4"`, `provider = "mistral"`,
/// `alias = "large-4"`), or `None` when
/// nothing is missing or the text is not a config with a `[[models]]` array. Keys already
/// present keep their value, whatever it is; other model entries are not touched.
fn with_missing_defaults(original: &str) -> Option<String> {
    let mut config: DocumentMut = original.parse().ok()?;
    // A person's own global compaction point is theirs for every model: Vibe takes a model's own point
    // over it, so our entry then gets none.
    let global_threshold = config.contains_key("auto_compact_threshold");
    let models = config.get_mut("models")?.as_array_of_tables_mut()?;
    let mut changed = false;
    for model in models.iter_mut() {
        if model.get("name").and_then(|item| item.as_str()) != Some("mistral-large-4")
            || model.get("provider").and_then(|item| item.as_str()) != Some("mistral")
            || model.get("alias").and_then(|item| item.as_str()) != Some("large-4")
        {
            continue;
        }
        if model
            .get("display_name")
            .is_none_or(|name| matches!(name.as_str(), Some("Large 4" | "large-4")))
        {
            let decor = model
                .get("display_name")
                .and_then(|item| item.as_value())
                .map(|v| v.decor().clone());
            model["display_name"] = value("Mistral Large 4");
            if let Some(decor) = decor {
                *model["display_name"].as_value_mut()?.decor_mut() = decor;
            }
            changed = true;
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
        if !global_threshold && !model.contains_key("auto_compact_threshold") {
            model["auto_compact_threshold"] = value(AUTO_COMPACT_THRESHOLD);
            changed = true;
        }
    }
    changed.then(|| config.to_string())
}

#[cfg(test)]
mod catalog_tests {
    use super::{ModelDefaults, edit};

    // PREDICT: the startup editor still writes the local Large 4 model and pins it as the active
    // default even when the catalog is enabled. A later catalog cannot become the model source.
    #[test]
    fn catalog_enabled_fresh_start_has_no_app_model_pin() {
        let (text, outcome) = edit("experimental_enable_model_catalog = true\n", false);
        assert_eq!(outcome, ModelDefaults::Unchanged);
        assert!(text.is_none());
    }

    // PREDICT: a previous app-owned Large 4 row remains in config.toml and overrides the catalog.
    #[test]
    fn catalog_enabled_upgrade_retires_only_the_exact_app_seed() {
        let old = format!(
            "experimental_enable_model_catalog = true\n{}",
            super::FIRST_CONFIG
        );
        let (text, outcome) = edit(&old, false);
        assert_eq!(outcome, ModelDefaults::Filled);
        let next: toml::Table = text.unwrap().parse().unwrap();
        assert_eq!(next["active_model"].as_str(), Some("large-4"));
        assert!(!next.contains_key("models"));

        let legacy = old
            .replace(
                "max_context_length = 1048576",
                "max_context_length = 1000000",
            )
            .replace("input_price = 0.7", "input_price = 0.68")
            .replace("output_price = 2.1", "output_price = 2.09");
        let (text, outcome) = edit(&legacy, false);
        assert_eq!(outcome, ModelDefaults::Filled);
        let next: toml::Table = text.unwrap().parse().unwrap();
        assert_eq!(next["active_model"].as_str(), Some("large-4"));
        assert!(!next.contains_key("models"));

        let custom = old.replace(
            "display_name = \"Mistral Large 4\"",
            "display_name = \"Mine\"",
        );
        let (text, outcome) = edit(&custom, false);
        assert_eq!(outcome, ModelDefaults::Unchanged);
        assert!(text.is_none());

        let off = old.replace(
            "experimental_enable_model_catalog = true",
            "experimental_enable_model_catalog = false",
        );
        let (text, outcome) = edit(&off, false);
        assert_eq!(outcome, ModelDefaults::Unchanged);
        assert!(text.is_none());
    }
}
