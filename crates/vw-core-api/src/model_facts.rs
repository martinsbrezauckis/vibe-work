//! Vibe Work's own facts for Mistral Large 4, for what Mistral's own Large 4 entry does not report: the window the
//! Mistral API reports (1,048,576 tokens) and the published prices. They live in one `[[models]]` entry for Mistral's alias in Vibe
//! Work's `config.toml`, marked by a comment and its shape (an alias and nothing but these facts: no `name`, no
//! `provider`); only an entry with both is Vibe Work's, so a person's own entry of the same shape is never touched.
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

/// True for the entry Vibe Work wrote: its marking comment, `alias` plus at least one fact and nothing else. The
/// comment is required: a person's own override of the same shape (an alias with a window, say) is never taken.
fn is_facts(model: &Table) -> bool {
    model
        .decor()
        .prefix()
        .and_then(|prefix| prefix.as_str())
        .is_some_and(|prefix| prefix.contains(MARK.trim_end()))
        && model.get("alias").and_then(Item::as_str).is_some()
        && model.len() > 1
        && model
            .iter()
            .all(|(key, _)| key == "alias" || FACT_KEYS.contains(&key))
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
        // Without its comment our entry is just an entry of that shape: it stays.
        let unmarked = reread.replace(MARK.trim_end(), "");
        assert_eq!(without(&unmarked), None);
    }
}
