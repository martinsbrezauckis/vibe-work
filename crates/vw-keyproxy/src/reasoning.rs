//! Thinking Off sends `reasoning_effort: "none"` to the Mistral reasoning models that accept it.
//! Called on chat JSON by `compaction::prepare`; explicit provider values always win (Vibe's adapter:
//! `vibe/adapters/mistral.py:80-85,135-138`). Unknown models and hosted GLM get no field: picker levels
//! alone do not prove they accept `none`.
//! Mistral capability reference: <https://docs.mistral.ai/studio/conversations/reasoning>

use serde_json::{Value, json};

pub(crate) fn explicit_off(request: &mut Value) -> bool {
    let Some(model) = request.get("model").and_then(Value::as_str) else {
        return false;
    };
    let supported = matches!(
        model,
        "mistral-large-4"
            | "mistral-large-4-0"
            | "mistral-small-latest"
            | "mistral-small-2603"
            | "mistral-medium-latest"
            | "mistral-medium-3-5"
    );
    if !supported
        || request
            .get("reasoning_effort")
            .is_some_and(|value| !value.is_null())
    {
        return false;
    }
    request["reasoning_effort"] = json!("none");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_medium_alias_disables_reasoning_without_guessing_a_date_id() {
        let mut body = json!({"model":"mistral-medium-latest"});
        assert!(explicit_off(&mut body));
        assert_eq!(body["reasoning_effort"], "none");
        let mut unknown = json!({"model":"mistral-medium-2604"});
        assert!(!explicit_off(&mut unknown));
        assert!(unknown.get("reasoning_effort").is_none());
    }

    #[test]
    fn explicit_values_and_models_without_a_none_capability_are_unchanged() {
        for mut body in [
            json!({"model":"mistral-large-4","reasoning_effort":"high"}),
            json!({"model":"mistral-large-4","reasoning_effort":"none"}),
            json!({"model":"zai-glm-5-3"}),
            json!({"model":"mistral-large-4-0-3"}),
            json!({"model":"mistral-vibe-cli-latest"}),
            json!({"model":"custom"}),
            json!({"messages":[]}),
        ] {
            let original = body.clone();
            assert!(!explicit_off(&mut body));
            assert_eq!(body, original);
        }
    }
}
