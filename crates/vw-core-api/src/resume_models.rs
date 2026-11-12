//! Core's trusted side of resume model admission; the proxy owns all HTTP enforcement.
//! Called by resume paths, the store pump and Stop. No model-provided value can authorize a turn.
use std::time::Duration;

use serde_json::Value;
use vw_session::Change;
use vw_supervisor::{ModelRequestHold, ModelRequests};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

pub(crate) fn begin(
    shared: &Shared,
    id: &str,
    bound: Duration,
) -> Result<Option<Box<dyn ModelRequestHold>>, CoreError> {
    let Some((turn, inputs)) = shared.interrupted_send.model_hold(id) else {
        return Ok(None);
    };
    let requests = shared.model_requests.as_ref().ok_or_else(|| failure(
        "The model proxy cannot protect this interrupted chat. Restart Vibe Work before sending again.",
    ))?;
    // A person's slow retry has its own budget; expiring at the ordinary reopen limit
    // would reject a proven terminal turn when that longer resume finally answers.
    // The proxy independently caps every hold at 60 seconds and fails closed.
    requests
        .hold(id, &turn, &inputs, bound)
        .map(Some)
        .map_err(|error| failure(&error.0))
}

pub(crate) fn release(hold: &mut Option<Box<dyn ModelRequestHold>>) -> Result<(), CoreError> {
    if let Some(hold) = hold {
        hold.release().map_err(|error| failure(&error.0))?;
    }
    Ok(())
}

fn failure(message: &str) -> CoreError {
    CoreError::new(ErrorCode::Denied, message).with_method("session/resume")
}

fn turn(requests: &dyn ModelRequests, id: &str, turn: &Value) {
    if turn["status"] == "in_progress"
        && let Some(turn_id) = turn["id"].as_str()
    {
        requests.new_turn(id, turn_id, turn["inputEntryId"].as_str());
    }
}

fn entry(requests: &dyn ModelRequests, id: &str, entry: &Value) {
    if entry["type"] == "message"
        && entry["role"] == "user"
        && let (Some(turn), Some(input)) = (entry["turnId"].as_str(), entry["id"].as_str())
    {
        requests.user_input(id, turn, input);
    }
}

pub(crate) fn observe(shared: &Shared, change: &Change) {
    let Some(requests) = &shared.model_requests else {
        return;
    };
    let requests = requests.as_ref();
    match change {
        Change::TurnUpserted {
            session_id,
            turn: value,
        } => turn(requests, session_id, value),
        Change::EntryUpserted {
            session_id,
            entry: value,
            ..
        } => entry(requests, session_id, value),
        Change::Replaced { session_id } => {
            if let Some(view) = shared.store.view(session_id) {
                state(shared, session_id, &view.state);
            }
        }
        _ => {}
    }
}

/// Live recovery events precede loading the resumed snapshot and can have reset event numbers.
/// Call after the pump verifies channel ownership, before the store's history watermark filtering.
pub(crate) fn notification(shared: &Shared, method: &str, params: &Value) {
    let Some(requests) = &shared.model_requests else {
        return;
    };
    let Some(id) = params["sessionId"].as_str() else {
        return;
    };
    match method {
        "turn/started" => turn(requests.as_ref(), id, &params["turn"]),
        "history/entryAdded" => entry(requests.as_ref(), id, &params["entry"]),
        _ => {}
    }
}

pub(crate) fn state(shared: &Shared, id: &str, state: &Value) {
    let Some(requests) = &shared.model_requests else {
        return;
    };
    for value in state["turns"].as_array().into_iter().flatten() {
        turn(requests.as_ref(), id, value);
    }
    for value in state["history"].as_array().into_iter().flatten() {
        entry(requests.as_ref(), id, value);
    }
}
