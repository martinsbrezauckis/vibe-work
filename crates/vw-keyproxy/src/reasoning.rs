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
