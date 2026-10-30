//! Fits Vibe's thinking level to what the Mistral reasoning models take. They do not take the same levels and
//! the set changes on the live API (Large 4 refuses `none` but takes low, medium and high; Medium and Small take
//! only `none` and `high`). A request without the field is taken by all of them and is Off on Medium and Small, as
//! Vibe's own adapter sends Off (`vibe/adapters/mistral.py:80-85,135-138`). So Off and `none` go out without the
//! field everywhere; Large 4 gets low, medium and high as picked (Mistral's own Large 4 entry offers those three),
//! Medium and Small get low without the field and medium as `high`; max and xhigh go out as `high`. A level kept in
//! an older config never fails a turn.
//! Called on chat JSON by `compaction::prepare`; other explicit values pass unchanged. Unknown models and hosted
//! GLM are left alone: picker levels alone do not prove what they take. That includes the new Flash models
//! `deepseek-v4-1-flash-1017` and `zai-glm-5-3-flash`: Mistral's model list marks both as reasoning
//! models but neither the list nor Mistral's reasoning page names the levels they take, so the level Vibe sends goes
//! out as it is.
//! Mistral capability reference: <https://docs.mistral.ai/studio/conversations/reasoning>
use serde_json::{Value, json};

pub(crate) fn fit(request: &mut Value) -> bool {
    let Some(model) = request.get("model").and_then(Value::as_str) else {
        return false;
    };
    let large_4 = matches!(model, "mistral-large-4" | "mistral-large-4-0");
    let supported = large_4
        || matches!(
            model,
            "mistral-small-latest"
                | "mistral-small-2603"
                | "mistral-medium-latest"
                | "mistral-medium-3-5"
        );
    if !supported {
        return false;
    }
    let sent = match request.get("reasoning_effort") {
        None => return false,
        Some(Value::Null) => None,
        Some(level) => match level.as_str() {
            Some("low" | "medium" | "high") if large_4 => return false,
            Some("none" | "low") => None,
            Some("medium" | "max" | "xhigh") => Some("high"),
            _ => return false,
        },
    };
    match sent {
        Some(level) => request["reasoning_effort"] = json!(level),
        None => {
            if let Some(fields) = request.as_object_mut() {
                fields.remove("reasoning_effort");
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live API: Large 4 now refuses `none` (it takes low, medium and high) while Medium and Small still take only
    /// `none` and `high`; every one of them takes a request without the field, which is Off on Medium and Small.
    #[test]
    fn off_sends_no_level_so_every_model_takes_it() {
        for model in [
            "mistral-large-4",
            "mistral-medium-latest",
            "mistral-small-latest",
        ] {
            let mut absent = json!({"model":model});
            assert!(!fit(&mut absent), "{model}");
            assert!(absent.get("reasoning_effort").is_none(), "{model}");
            for off in [json!(null), json!("none")] {
                let mut body = json!({"model":model,"reasoning_effort":off});
                assert!(fit(&mut body), "{model}");
                assert!(body.get("reasoning_effort").is_none(), "{model}");
            }
        }
    }

    /// A level some of these models refuse goes out as one they take (checked on the live API).
    #[test]
    fn a_level_these_models_refuse_is_sent_as_the_nearest_one_they_take() {
        for (level, sent) in [
            ("low", None),
            ("medium", Some("high")),
            ("max", Some("high")),
            ("xhigh", Some("high")),
        ] {
            for model in ["mistral-medium-3-5", "mistral-small-latest"] {
                let mut body = json!({"model":model,"reasoning_effort":level});
                assert!(fit(&mut body), "{model} {level}");
                assert_eq!(
                    body.get("reasoning_effort").and_then(Value::as_str),
                    sent,
                    "{model} {level}"
                );
            }
        }
        let mut unknown = json!({"model":"custom","reasoning_effort":"low"});
        assert!(!fit(&mut unknown));
        assert_eq!(unknown["reasoning_effort"], "low");
    }

    #[test]
    fn explicit_values_and_models_without_a_none_capability_are_unchanged() {
        for mut body in [
            json!({"model":"mistral-large-4","reasoning_effort":"high"}),
            json!({"model":"mistral-large-4"}),
            json!({"model":"zai-glm-5-3"}),
            json!({"model":"zai-glm-5-3-flash","reasoning_effort":"low"}),
            json!({"model":"deepseek-v4-1-flash-1017","reasoning_effort":"medium"}),
            json!({"model":"deepseek-v4-1-flash-1017","reasoning_effort":null}),
            json!({"model":"mistral-large-4-0-3"}),
            json!({"model":"mistral-vibe-cli-latest"}),
            json!({"model":"custom"}),
            json!({"messages":[]}),
        ] {
            let original = body.clone();
            assert!(!fit(&mut body));
            assert_eq!(body, original);
        }
    }
}
