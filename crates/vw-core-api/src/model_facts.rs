//! Vibe Work's own facts for Mistral Large 4, for what Mistral's own Large 4 entry does not report: the window the
//! Mistral API reports (1,048,576 tokens) and the published prices. They live in one `[[models]]` entry for Mistral's alias in Vibe
//! Work's `config.toml`, marked by a comment and its shape (an alias and nothing but these facts: no `name`, no
//! `provider`). An unmarked entry is Vibe Work's only for Mistral's Large 4 aliases with every price at ours and
//! the window, if present, at ours; a person's entry for another alias or with a value of their own stays theirs.
//! Vibe deep-merges it onto Mistral's entry, so Vibe's compaction, the context meter and the session's prices keep
//! working.
//!
//! Mistral's own values win: the launch check (`model_check`) reads Mistral's entry with this entry taken out, and
//! writes back only the facts Mistral's entry still lacks. Prices are never in Vibe's model views
//! (`app_server/config.py` `ModelConfigView`), so ours always apply.
//!
//! Depends on: `toml_edit`. Called by: `model_check`.

use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table, value};

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
const MARK: &str = "# Vibe Work's facts for Mistral's Large 4 entry where Mistral reports none (Mistral's own values win).\n";

/// True for the entry Vibe Work wrote: `alias` plus at least one fact and nothing else, and either its marking
/// comment or a Mistral Large 4 alias with exactly the values `with` writes (every price, and the window if there
/// is one): a person's override for another alias or with a value of their own is never taken.
fn is_facts(model: &Table) -> bool {
    let shaped = model.get("alias").and_then(Item::as_str).is_some()
        && model.len() > 1
        && model
            .iter()
            .all(|(key, _)| key == "alias" || FACT_KEYS.contains(&key));
    let marked = model
        .decor()
        .prefix()
        .and_then(|prefix| prefix.as_str())
        .is_some_and(|prefix| prefix.contains(MARK.trim_end()));
    shaped && (marked || holds_exactly_ours(model))
}

/// A Mistral Large 4 alias with every price at the value `with` writes, and the window (when present) at ours.
fn holds_exactly_ours(model: &Table) -> bool {
    matches!(
        model.get("alias").and_then(Item::as_str),
        Some("mistral-large-4" | "mistral-large-4-0" | "mistral-large-4-0-3")
    ) && PRICES
        .iter()
        .all(|(key, price)| model.get(key).and_then(Item::as_float) == Some(*price))
        && model
            .get("max_context_length")
            .is_none_or(|window| window.as_integer() == Some(WINDOW))
}

/// `text` without Vibe Work's facts entry, or `None` when it has none.
pub(crate) fn without(text: &str) -> Option<String> {
    let mut config: DocumentMut = text.parse().ok()?;
    let models = config
        .get_mut("models")
        .and_then(Item::as_array_of_tables_mut)?;
    let before = models.len();
    models.retain(|model| !is_facts(model));
    (models.len() != before).then(|| config.to_string())
}

/// `text` (without a facts entry) with one for `alias`, holding the window when Mistral's `row` reports none and
/// the prices; `text` unchanged when it cannot be parsed.
pub(crate) fn with(text: &str, alias: &str, row: &Value) -> String {
    let Ok(mut config) = text.parse::<DocumentMut>() else {
        return text.to_owned();
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
    if !config.contains_key("models") {
        config["models"] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    if let Some(models) = config["models"].as_array_of_tables_mut() {
        models.push(facts);
    }
    config.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PERSONS: &str = "[[models]]\nalias = \"my-model\"\nmax_context_length = 131072\n\n[[models]]\nalias = \"priced\"\ninput_price = 1.5\n";

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
                assert_eq!(without(&theirs), None, "{theirs}");
                let combined = with(&theirs, "mistral-large-4-0", &json!({}));
                assert_eq!(without(&combined).as_deref(), Some(theirs.as_str()));
            }
        }
    }

    #[test]
    fn a_persons_entry_of_the_same_shape_is_never_taken_for_ours() {
        assert_eq!(without(PERSONS), None, "nothing of ours to take out");
        let ours = with(PERSONS, "mistral-large-4-0", &json!({}));
        let bare = without(&ours).expect("our marked entry is found");
        assert_eq!(
            bare, PERSONS,
            "only our entry left, every byte of the person's kept"
        );
    }

    #[test]
    fn our_marked_entry_is_recognised_after_a_round_trip() {
        let ours = with(
            "active_model = \"large-4\"\n",
            "mistral-large-4-0",
            &json!({}),
        );
        assert!(ours.contains(MARK.trim_end()), "{ours}");
        let reread = ours.parse::<DocumentMut>().unwrap().to_string();
        assert_eq!(
            without(&reread).as_deref(),
            Some("active_model = \"large-4\"\n")
        );
        // Without its comment and with a value of the person's own it is just an entry of that shape: it stays
        // (with exactly our values it is still ours: `an_unmarked_entry_with_exactly_our_values_is_still_ours`).
        let unmarked = reread
            .replace(MARK.trim_end(), "")
            .replace("input_price = 0.7", "input_price = 0.8");
        assert_eq!(without(&unmarked), None);
    }

    #[test]
    fn a_marked_entry_with_an_account_specific_alias_is_still_ours() {
        let ours = with("", "account-large-4", &json!({"maxContextLength": 500_000}));
        assert_eq!(without(&ours).as_deref(), Some(""));
        let changed_price = ours.replace("input_price = 0.7", "input_price = 0.8");
        assert_eq!(without(&changed_price).as_deref(), Some(""));
        assert_eq!(without(&ours.replace(MARK.trim_end(), "")), None);
    }

    /// An unmarked entry for a Mistral Large 4 alias with exactly our keys and values (with or without the window)
    /// is still ours; one value of the person's own makes it theirs.
    #[test]
    fn an_unmarked_entry_with_exactly_our_values_is_still_ours() {
        let base = "active_model = \"large-4\"\n";
        for alias in [
            "mistral-large-4",
            "mistral-large-4-0",
            "mistral-large-4-0-3",
        ] {
            let ours = with(base, alias, &json!({}));
            let unmarked = ours.replace(MARK.trim_end(), "");
            assert_eq!(without(&unmarked).as_deref(), Some(base), "{unmarked}");
            let windowed = with(base, alias, &json!({"maxContextLength": 500_000}));
            let unmarked = windowed.replace(MARK.trim_end(), "");
            assert!(!unmarked.contains("max_context_length"), "{unmarked}");
            assert_eq!(without(&unmarked).as_deref(), Some(base), "{unmarked}");
            for theirs in [
                unmarked.replace("output_price = 2.1", "output_price = 2.5"),
                format!("{unmarked}max_output_tokens = 1000\n"),
                ours.replace(MARK.trim_end(), "").replace(
                    "max_context_length = 1048576",
                    "max_context_length = 131072",
                ),
            ] {
                assert_eq!(without(&theirs), None, "{theirs}");
            }
        }
    }
}
