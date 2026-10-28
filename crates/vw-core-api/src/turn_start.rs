//! Bridges the gap between Vibe publishing an idle turn and releasing its active task.
//! Uses the gate and session store; dispatch retries only a conflict naming that exact finished turn.
//! Authorized failed deletes are completed by `session_delete`, without resending a destructive request.
//! Also the one place every forwarded call passes: it records a chat's work as started before sending it, and
//! a checked app continuation's Stop test runs here, right before each attempt is sent (`stops.rs`).

use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;
use vw_gate::{ConfirmationToken, GateError, Reply};
use vw_rpc::RpcError;

use crate::core::Shared;

pub(crate) async fn call(
    shared: &Shared,
    method: &str,
    params: &Value,
    token: Option<&ConfirmationToken>,
    stop: Option<u64>,
) -> Result<Reply, GateError> {
    if method == "session/delete" {
        return shared.gate.call(method, params.clone(), token).await;
    }
    // Work is marked started before it is sent (any doubt = started); only a refusal from before the send undoes it
    // (`plugin_refresh::unstarted`). Dropping this future keeps the mark.
    let admission = shared.plugin_refresh.admission(method, params);
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut retry = false;
    loop {
        // No await between this test and the send it guards: a Stop after it waits for the chat's send lock.
        if let Err(stopped) = crate::stops::sendable(shared, stop, method, params) {
            // Only a first attempt never left the core; a retry follows one Vibe answered.
            if !retry {
                admission.refused_by(&stopped);
            }
            return Err(stopped);
        }
        let at_send = params["sessionId"]
            .as_str()
            .map_or(0, |id| crate::stops::generation(shared, id));
        let result = shared.gate.call(method, params.clone(), token).await;
        if let Ok(reply) = &result {
            crate::stops::sent(shared, stop, at_send, method, params, &reply.result);
        }
        let closing = method == "turn/start"
            && token.is_none()
            && result.as_ref().err().is_some_and(|error| {
                params["sessionId"]
                    .as_str()
                    .and_then(|id| shared.store.view(id))
                    .is_some_and(|view| finished_conflict(&view.state, error))
            });
        if !closing || Instant::now() >= deadline {
            if let Err(error) = &result {
                admission.refused_by(error);
            }
            return result;
        }
        retry = true;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn finished_conflict(state: &Value, error: &GateError) -> bool {
    let GateError::Transport(RpcError::Remote { error, .. }) = error else {
        return false;
    };
    let Some(data) = error.data.as_option() else {
        return false;
    };
    if error.code.as_str() != "conflict"
        || data["harnessCode"] != "turn_conflict"
        || state["session"]["status"]["type"] != "idle"
    {
        return false;
    }
    let Some(active) = data["details"]["active_turn_id"].as_str() else {
        return false;
    };
    state["turns"]
        .as_array()
        .and_then(|turns| turns.last())
        .is_some_and(|turn| {
            turn["id"] == active
                && matches!(
                    turn["status"].as_str(),
                    Some("completed" | "failed" | "interrupted" | "cancelled")
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_idle_sessions_exact_finished_task_can_be_retried() {
        let error = |code: &str, turn: &str| {
            GateError::Transport(RpcError::Remote {
                method: "turn/start".into(),
                error: serde_json::from_value(json!({"code":code,"message":"conflict","data":{
                    "harnessCode":"turn_conflict","details":{"active_turn_id":turn}
                }}))
                .unwrap(),
            })
        };
        let mut state = json!({"session":{"status":{"type":"idle"}},"turns":[{"id":"finished","status":"completed"}]});
        assert!(finished_conflict(&state, &error("conflict", "finished")));
        assert!(!finished_conflict(&state, &error("conflict", "another")));
        assert!(!finished_conflict(
            &state,
            &error("internal_error", "finished")
        ));
        state["session"]["status"]["type"] = json!("running");
        assert!(!finished_conflict(&state, &error("conflict", "finished")));
        state["session"]["status"]["type"] = json!("idle");
        state["turns"][0]["status"] = json!("in_progress");
        assert!(!finished_conflict(&state, &error("conflict", "finished")));
    }
}
