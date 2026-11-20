//! Recognizes provider rejection of an agent's selected model, independently of catalog retirement.
//! Uses Vibe's structured completion error; model recovery runs only after the failed turn ends.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use vw_platform::sync::LockRecover;

use crate::core::Shared;

fn rejected_model(turn: &Value) -> Option<&str> {
    let error = &turn["error"];
    let details = &error["details"];
    if turn["status"] != "failed"
        || error["code"] != "backend_error"
        || details["exceptionType"] != "SDKError"
        || details["provider"] != "mistral"
        || details["purpose"] != "agent"
        || !matches!(details["httpStatus"].as_u64(), Some(400 | 404))
    {
        return None;
    }
    let message = error["message"].as_str()?.to_ascii_lowercase();
    if ![
        "model not found",
        "unknown model",
        "model does not exist",
        "model is not available",
    ]
    .iter()
    .any(|phrase| message.contains(phrase))
    {
        return None;
    }
    details["model"].as_str().filter(|name| !name.is_empty())
}

/// A failed provider request may replace a model only after its turn has ended, under the chat's command lock.
pub(crate) fn completed(shared: &Arc<Shared>, params: &Value) {
    let Some(model) = rejected_model(&params["turn"]) else {
        return;
    };
    let (Some(id), Some(turn)) = (params["sessionId"].as_str(), params["turn"]["id"].as_str())
    else {
        return;
    };
    let Some(choice) = shared
        .store
        .runtime(id)
        .and_then(|runtime| super::actual(&json!({"runtime":runtime})))
        .filter(|choice| choice.name == model)
    else {
        return;
    };
    {
        let mut rejected = shared.model_choices.rejected_turns.lock_or_recover();
        if rejected.get(id).is_some_and(|previous| previous == turn) {
            return;
        }
        rejected.insert(id.to_owned(), turn.to_owned());
    }
    let (shared, id, turn) = (Arc::clone(shared), id.to_owned(), turn.to_owned());
    tokio::spawn(async move {
        let _commands = shared.plugin_refresh.lock(&id).await;
        let _restore = shared.access.lock_restore(&id).await;
        if !shared.channels.has_channel(&id) || shared.channels.is_restoring(&id) {
            return;
        }
        // The completion notification can precede the session's final idle patch.
        let mut idle = false;
        for _ in 0..100 {
            let Some(view) = shared.store.view(&id) else {
                return;
            };
            let turns = view.state["turns"].as_array();
            let latest =
                turns.and_then(|turns| turns.iter().max_by_key(|turn| turn["startedAt"].as_i64()));
            if latest.is_none_or(|latest| latest["id"].as_str() != Some(&turn)) {
                return;
            }
            let status = &view.state["session"]["status"];
            let status = status.as_str().or_else(|| status["type"].as_str());
            let running = matches!(status, Some("running" | "blocked"))
                || turns
                    .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "in_progress"));
            if !running {
                idle = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if !idle {
            return;
        }
        let Ok(runtime) = super::restore::read(&shared, &id).await else {
            return;
        };
        if super::actual(&json!({"runtime":runtime}))
            .is_none_or(|current| !choice.matches(&current))
        {
            return;
        }
        let runtime = super::restore::fallback(&shared, &id, &choice, runtime).await;
        super::publish(&shared, &id, runtime).await;
    });
}

#[cfg(test)]
mod tests {
    use super::rejected_model;
    use serde_json::{Value, json};

    fn failed(status: u16, message: &str) -> Value {
        json!({"id":"rejected-turn","status":"failed","error":{
            "code":"backend_error","message":message,
            "details":{"exceptionType":"SDKError","httpStatus":status,
                "model":"catalog-model","provider":"mistral","purpose":"agent"}}})
    }

    // PREDICT: an explicit model-unavailable completion is admitted for recovery; unrelated errors are not.
    #[test]
    fn recognizes_the_observed_model_rejection() {
        let turn = failed(
            404,
            "API error occurred: Status 404. Body: {\"message\":\"Model not found in Mistral provider\"}",
        );
        assert_eq!(rejected_model(&turn), Some("catalog-model"));
        assert_eq!(
            rejected_model(&failed(400, "Unknown model id: catalog-model")),
            Some("catalog-model")
        );
    }

    #[test]
    fn authentication_rate_limits_network_and_generic_failures_keep_the_pick() {
        for status in [401, 403, 408, 429, 500, 502, 503] {
            assert_eq!(rejected_model(&failed(status, "Model not found")), None);
        }
        for message in [
            "",
            "Not found",
            "Context length exceeded",
            "Invalid reasoning effort",
            "Tool schema is invalid",
            "The model exceeded its context window",
        ] {
            for status in [400, 404] {
                assert_eq!(
                    rejected_model(&failed(status, message)),
                    None,
                    "{status}: {message}"
                );
            }
        }
    }

    #[test]
    fn side_completions_missing_details_and_nonterminal_turns_keep_the_pick() {
        let original = failed(404, "Model not found");
        for (pointer, value) in [
            ("/status", json!("in_progress")),
            ("/error/code", json!("unknown")),
            ("/error/details/purpose", json!("title")),
            ("/error/details/provider", json!("other-provider")),
            ("/error/details/model", json!("")),
            ("/error/details/model", Value::Null),
            ("/error/details/httpStatus", Value::Null),
            ("/error/details/exceptionType", json!("ConnectionError")),
        ] {
            let mut turn = original.clone();
            *turn.pointer_mut(pointer).unwrap() = value;
            assert_eq!(rejected_model(&turn), None, "{pointer}");
        }
        assert_eq!(rejected_model(&json!({})), None);
        assert_eq!(
            rejected_model(&json!({"status":"failed","error":null})),
            None
        );
    }
}
