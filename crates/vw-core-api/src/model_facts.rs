//! Vibe Work's own facts for Mistral Large 4, for what Mistral's own Large 4 entry does not report: the window the
//! Mistral API reports (1,048,576 tokens) and the published prices. They live in one `[[models]]` entry for Mistral's alias in Vibe
//! Work's `config.toml`, identified by the exact fields and values we write (no `name` or `provider`). Vibe's own
//! config write drops comments, so known Large 4 aliases are recognised without the comment or side record.
//! Entries with different content are the person's and keep every byte. Vibe deep-merges the facts entry onto
//! Mistral's entry, so Vibe's compaction, the context
//! meter and the session's prices keep working.
//!
//! Mistral's own values win: the launch check (`model_check`) reads Mistral's entry with this entry taken out, and
//! writes back only the facts Mistral's entry still lacks. Prices are never in Vibe's model views
//! (`app_server/config.py` `ModelConfigView`), so ours always apply.
//!
//! Depends on: `toml_edit`, the private atomic writer. Called by: `model_check`.
//! Invariant: the record never names an entry the file may lack: it is cleared before a write that takes our entry
//! out and written only after a write that puts it in. A record may identify an account-specific alias, but never
//! authorizes removing different content.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::error::{CoreError, ErrorCode};

// The API's window and the prices published at https://docs.mistral.ai/models/mistral-large-4-0 (USD per million
// tokens), the same values Vibe Work's own Large 4 entry carries (`vw_host::default_model`).
const WINDOW: i64 = 1_048_576;
const PRICES: &[(&str, f64)] = &[
    ("input_price", 0.70),
    ("cached_input_price", 0.07),
    ("output_price", 2.10),
];
const FACT_KEYS: &[&str] = &[
    "max_context_length",
    "input_price",
    "cached_input_price",
    "output_price",
];
pub(crate) const MARK: &str = "# Vibe Work's facts for Mistral's Large 4 entry where Mistral reports none (Mistral's own values win).\n";

/// The record of the facts entry Vibe Work last wrote, in Vibe Work's own Vibe home beside `config.toml`.
const RECORD: &str = ".vibe-work-model-facts.json";

/// The facts entry Vibe Work wrote, as its keys and values (`alias` and each fact).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Written(Map<String, Value>);

/// The alias plus at least one fact and nothing else: the only shape Vibe Work writes.
fn shaped(model: &Table) -> bool {
    model.get("alias").and_then(Item::as_str).is_some()
        && model.len() > 1
        && model
            .iter()
            .all(|(key, _)| key == "alias" || FACT_KEYS.contains(&key))
}

fn marked(model: &Table) -> bool {
    shaped(model)
        && model
            .decor()
            .prefix()
            .and_then(|prefix| prefix.as_str())
            .is_some_and(|prefix| prefix.contains(MARK.trim_end()))
}

/// A shaped entry's keys and values in the record's terms; `None` for a value of another type than ours.
fn values(model: &Table) -> Option<Map<String, Value>> {
    model
        .iter()
        .map(|(key, item)| {
            let value = match key {
                "alias" => Value::from(item.as_str()?),
                "max_context_length" => Value::from(item.as_integer()?),
                _ => Value::from(item.as_float()?),
            };
            Some((key.to_owned(), value))
        })
        .collect()
}

fn ours(model: &Table, written: Option<&Written>) -> bool {
    let Some(alias) = model.get("alias").and_then(Item::as_str) else {
        return false;
    };
    let known = [
        "mistral-large-4",
        "mistral-large-4-0",
        "mistral-large-4-0-3",
    ]
    .contains(&alias);
    let recorded = written.is_some_and(|Written(record)| values(model).as_ref() == Some(record));
    (known || marked(model) || recorded)
        && shaped(model)
        && PRICES
            .iter()
            .all(|(key, price)| model.get(key).and_then(Item::as_float) == Some(*price))
        && model
            .get("max_context_length")
            .is_none_or(|item| item.as_integer() == Some(WINDOW))
}

/// `text` without our exact facts entries, including those whose comments Vibe stripped.
pub(crate) fn without(text: &str, written: Option<&Written>) -> Option<String> {
    let mut config: DocumentMut = text.parse().ok()?;
    let models = config
        .get_mut("models")
        .and_then(Item::as_array_of_tables_mut)?;
    let before = models.len();
    models.retain(|model| !ours(model, written));
    (models.len() != before).then(|| config.to_string())
}

/// True when `text` has an entry of its own for `alias` (Vibe takes an entry's `name` when it has no alias). Vibe
/// keeps only the last entry of an alias, so a facts entry added after it would replace the person's.
pub(crate) fn has_entry_for(text: &str, alias: &str) -> bool {
    let Ok(config) = text.parse::<DocumentMut>() else {
        return true;
    };
    config
        .get("models")
        .and_then(Item::as_array_of_tables)
        .is_some_and(|models| {
            models.iter().any(|model| {
                model
                    .get("alias")
                    .or_else(|| model.get("name"))
                    .and_then(Item::as_str)
                    == Some(alias)
            })
        })
}

/// `text` (without a facts entry) with one for `alias`, holding the window when Mistral's `row` reports none and
/// the prices, and the record of that entry; `text` unchanged and no record when it cannot be parsed.
pub(crate) fn with(text: &str, alias: &str, row: &Value) -> (String, Option<Written>) {
    let Ok(mut config) = text.parse::<DocumentMut>() else {
        return (text.to_owned(), None);
    };
    let mut facts = Table::new();
    facts["alias"] = value(alias);
    if row["maxContextLength"]
        .as_u64()
        .is_none_or(|window| window == 0)
    {
        facts["max_context_length"] = value(WINDOW);
    }
    for (key, price) in PRICES {
        facts[*key] = value(*price);
    }
    facts.decor_mut().set_prefix(format!("\n{MARK}"));
    let written = values(&facts).map(Written);
    if !config.contains_key("models") {
        config["models"] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    if let Some(models) = config["models"].as_array_of_tables_mut() {
        models.push(facts);
    }
    (config.to_string(), written)
}

fn record_path(home: &Path) -> PathBuf {
    home.join(RECORD)
}

/// The recorded facts entry in Vibe home `home`; `None` when there is none or it cannot be read.
pub(crate) fn recorded(home: &Path) -> Option<Written> {
    let text = std::fs::read_to_string(record_path(home)).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(map) => Some(Written(map)),
        _ => None,
    }
}

/// Records `written` as the facts entry in Vibe home `home`, or clears the record for `None`.
///
/// # Errors
/// The record cannot be saved or removed.
pub(crate) fn record(home: &Path, written: Option<&Written>) -> Result<(), CoreError> {
    let path = record_path(home);
    let failed = || {
        CoreError::new(
            ErrorCode::Internal,
            "Vibe Work's model facts record could not be saved",
        )
    };
    match written {
        Some(Written(map)) => {
            let bytes = serde_json::to_vec(map).map_err(|_| failed())?;
            vw_platform::replace_private(&path, &bytes).map_err(|_| failed())
        }
        None => match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(failed()),
            _ => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PERSONS: &str = "[[models]]\nalias = \"my-model\"\nmax_context_length = 131072\n\n[[models]]\nalias = \"priced\"\ninput_price = 1.5\n";
    const ALIASES: [&str; 3] = [
        "mistral-large-4",
        "mistral-large-4-0",
        "mistral-large-4-0-3",
    ];

    #[test]
    fn a_different_alias_with_our_prices_is_never_taken_for_ours() {
        for alias in [
            "my-model",
            "large-4",
            "mistral-large-40",
            "mistral-large-4-custom",
            "mistral-large-4-0-30",
        ] {
            for window in ["", "max_context_length = 1048576\n"] {
                let theirs = format!(
                    "# My prices\n[[models]]\nalias = '{alias}'\n{window}input_price = 0.70 # keep this\ncached_input_price = 0.07\noutput_price = 2.10\n"
                );
                assert_eq!(without(&theirs, None), None, "{theirs}");
                let (combined, written) = with(&theirs, "mistral-large-4-0", &json!({}));
                assert_eq!(without(&combined, None).as_deref(), Some(theirs.as_str()));
                let unmarked = combined.replace(MARK.trim_end(), "");
                assert_eq!(
                    without(&unmarked, written.as_ref()).as_deref(),
                    Some(theirs.as_str()),
                    "{unmarked}"
                );
            }
        }
    }

    /// Different values on the same aliases belong to the person, even with a stale side record.
    #[test]
    fn a_persons_same_alias_entry_with_other_values_is_never_taken_for_ours() {
        for alias in ALIASES {
            for window in ["", "max_context_length = 1048576\n"] {
                let theirs = format!(
                    "[[models]]\nalias = \"{alias}\"\n{window}input_price = 0.8\ncached_input_price = 0.07\noutput_price = 2.1\n"
                );
                assert_eq!(without(&theirs, None), None, "{theirs}");
                assert!(has_entry_for(&theirs, alias));
                // A record of another alias, or of the same alias with the other window case.
                let other_window = if window.is_empty() {
                    json!({})
                } else {
                    json!({"maxContextLength": 1})
                };
                for other in ALIASES {
                    let row = if other == alias {
                        other_window.clone()
                    } else {
                        json!({})
                    };
                    let (_, written) = with("", other, &row);
                    assert_eq!(
                        without(&theirs, written.as_ref()),
                        None,
                        "{other}: {theirs}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_persons_entry_of_the_same_shape_is_never_taken_for_ours() {
        assert_eq!(without(PERSONS, None), None, "nothing of ours to take out");
        let (ours, written) = with(PERSONS, "mistral-large-4-0", &json!({}));
        let bare = without(&ours, written.as_ref()).expect("our marked entry is found");
        assert_eq!(
            bare, PERSONS,
            "only our entry left, every byte of the person's kept"
        );
    }

    #[test]
    fn our_marked_entry_is_recognised_after_a_round_trip() {
        let (ours, _) = with(
            "active_model = \"large-4\"\n",
            "mistral-large-4-0",
            &json!({}),
        );
        assert!(ours.contains(MARK.trim_end()), "{ours}");
        let reread = ours.parse::<DocumentMut>().unwrap().to_string();
        assert_eq!(
            without(&reread, None).as_deref(),
            Some("active_model = \"large-4\"\n")
        );
        // Exact facts remain ours after Vibe drops the comment and without a side record.
        assert!(without(&reread.replace(MARK.trim_end(), ""), None).is_some());
    }

    #[test]
    fn a_marked_entry_with_an_account_specific_alias_is_still_ours() {
        let (ours, written) = with("", "account-large-4", &json!({"maxContextLength": 500_000}));
        assert_eq!(without(&ours, None).as_deref(), Some(""));
        let changed_price = ours.replace("input_price = 0.7", "input_price = 0.8");
        assert_eq!(without(&changed_price, None), None);
        let unmarked = ours.replace(MARK.trim_end(), "");
        assert_eq!(without(&unmarked, None), None);
        assert_eq!(without(&unmarked, written.as_ref()).as_deref(), Some(""));
    }

    /// Exact known-alias facts survive serialization and need no side record; different content is untouched.
    #[test]
    fn our_exact_unmarked_entry_is_recognised_with_or_without_a_record() {
        let base = "active_model = \"large-4\"\n";
        for alias in ALIASES {
            for row in [json!({}), json!({"maxContextLength": 500_000})] {
                let (ours, written) = with(base, alias, &row);
                let written = written.expect("a record of what was written");
                let unmarked = ours.replace(MARK.trim_end(), "");
                assert_eq!(
                    without(&unmarked, None).as_deref(),
                    Some(base),
                    "{unmarked}"
                );
                assert_eq!(
                    without(&unmarked, Some(&written)).as_deref(),
                    Some(base),
                    "{unmarked}"
                );
                // Field order and serialization spelling do not change recognition.
                let rewritten = unmarked.parse::<toml::Table>().unwrap().to_string();
                assert!(without(&rewritten, None).is_some(), "{rewritten}");
                for theirs in [
                    unmarked.replace("output_price = 2.1", "output_price = 2.5"),
                    unmarked.replace("output_price = 2.1", "output_price = 2"),
                    format!("{unmarked}max_output_tokens = 1000\n"),
                    format!("{unmarked}name = 'personal-model'\n"),
                    format!("{unmarked}provider = 'personal-provider'\n"),
                    unmarked.replace("cached_input_price = 0.07", "cached_input_price = 0.08"),
                    unmarked.replace("cached_input_price = 0.07\n", ""),
                    unmarked.replace(&format!("alias = \"{alias}\""), "alias = \"other\""),
                ] {
                    assert_eq!(without(&theirs, Some(&written)), None, "{theirs}");
                }
                // Duplicate exact facts are both ours; neither may reach Vibe as an orphan.
                let entry = &unmarked[base.len()..];
                let twice = format!("{base}{entry}{entry}");
                assert_eq!(
                    without(&twice, Some(&written)).as_deref(),
                    Some(base),
                    "{twice}"
                );
            }
        }
    }

    #[test]
    fn the_record_round_trips_and_clears() {
        let home = std::env::temp_dir().join(format!("vw-facts-record-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(recorded(&home), None);
        let (_, written) = with("", "mistral-large-4-0", &json!({}));
        record(&home, written.as_ref()).unwrap();
        assert_eq!(recorded(&home), written);
        record(&home, None).unwrap();
        record(&home, None).unwrap();
        assert_eq!(recorded(&home), None);
        std::fs::write(home.join(RECORD), "not json").unwrap();
        assert_eq!(recorded(&home), None, "an unreadable record names nothing");
        std::fs::remove_dir_all(home).unwrap();
    }
}
