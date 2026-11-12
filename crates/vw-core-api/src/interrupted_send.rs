//! An explicit action or saved schedule stops an interrupted old turn before starting fresh work.
//! Uses ordinary gated resume/interrupt/read calls in the chat's own mode. Dispatch/recovery own the
//! chat lock; snapshots retain the old Interrupted row without inventing a Stop entry or hiding effects.
//! Depends on: gated resume, interrupt and read calls.
//! Called by: dispatch and saved-schedule recovery.
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use crate::core::{Core, Shared};
use crate::error::CoreError;
use crate::event::{CoreEvent, SessionChange};

struct Interrupted {
    turn: Value,
    inputs: Vec<String>,
    pending: bool,
    terminal_event: bool,
}

#[derive(Default)]
pub(crate) struct State(Mutex<HashMap<String, Interrupted>>);

impl State {
    /// Only actual turn events count, never a detached read's Interrupted projection.
    pub(crate) fn turn_event(&self, id: &str, turn: &Value) {
        if let Some(old) = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(id)
            && old.pending
            && old.turn["id"] == turn["id"]
        {
            old.terminal_event = matches!(
                turn["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            );
        }
    }

    fn terminal_event(&self, id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .is_some_and(|old| old.terminal_event)
    }

    pub(crate) fn model_hold(&self, id: &str) -> Option<(String, Vec<String>)> {
        let old = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let old = old.get(id).filter(|old| old.pending)?;
        Some((old.turn["id"].as_str()?.to_owned(), old.inputs.clone()))
    }

    pub(crate) fn pending(&self, id: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .filter(|old| old.pending)
            .and_then(|old| old.turn["id"].as_str().map(str::to_owned))
    }

    pub(crate) fn forget(&self, id: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }

    pub(crate) fn project(&self, id: &str, state: &mut Value) {
        let old = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(old) = old.get(id) else { return };
        if let Some(turns) = state["turns"].as_array_mut() {
            for turn in turns {
                if turn["id"] == old.turn["id"] {
                    turn.clone_from(&old.turn);
                }
            }
        }
        if state["session"]["status"]["activeTurnId"] == old.turn["id"] {
            state["session"]["status"] = json!({"type":"idle"});
        }
    }

    pub(crate) fn project_event(&self, event: &mut CoreEvent) {
        match event {
            CoreEvent::Snapshot { snapshot } => {
                self.project(&snapshot.session_id, &mut snapshot.state);
            }
            CoreEvent::Change { change } => {
                let old = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                match change {
                    SessionChange::TurnUpserted { session_id, turn } => {
                        if let Some(old) = old
                            .get(session_id)
                            .filter(|old| turn["id"] == old.turn["id"])
                        {
                            turn.clone_from(&old.turn);
                        }
                    }
                    SessionChange::SessionPatched { session_id, patch } => {
                        if let Some(old) = old.get(session_id) {
                            for op in patch {
                                if op["path"] == "/status"
                                    && op["value"]["activeTurnId"] == old.turn["id"]
                                {
                                    op["value"] = json!({"type":"idle"});
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Called under the chat lock before an explicit attach or a scheduled restart, including before plugin repair.
pub(crate) async fn prepare(core: &Core, id: &str) -> Result<(), CoreError> {
    if core.shared.channels.has_channel(id) {
        return Ok(());
    }
    let params = json!({"sessionId":id});
    let read =
        match core
            .shared
            .gate
            .call("session/read", params.clone(), None)
            .await
        {
            Ok(read) => read,
            Err(error) => {
                let error = CoreError::from_gate("session/read", &error);
                // Vibe does not persist an empty chat, even after rejecting its first message.
                // Only this core's known empty chat can have no saved old turn; other read errors fail closed.
                if error.server_code.as_deref() == Some("not_found")
                    && core.shared.plugin_refresh.created_here(id)
                    && core.shared.store.view(id).is_some_and(|view| {
                        view.state["turns"].as_array().is_some_and(Vec::is_empty)
                    })
                {
                    return Ok(());
                }
                return Err(error);
            }
        };
    let Some(turn) = read.result["state"]["turns"]
        .as_array()
        .and_then(|turns| turns.last())
        .filter(|turn| matches!(turn["status"].as_str(), Some("interrupted" | "in_progress")))
    else {
        if let Some(old) = core
            .shared
            .interrupted_send
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(id)
        {
            old.pending = false;
        }
        return Ok(());
    };
    let mut turn = turn.clone();
    let inputs = turn["inputEntryId"]
        .as_str()
        .into_iter()
        .chain(
            read.result["state"]["history"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| entry["turnId"] == turn["id"] && entry["role"] == "user")
                .filter_map(|entry| entry["id"].as_str()),
        )
        .map(str::to_owned)
        .collect();
    turn["status"] = json!("interrupted");
    core.shared
        .interrupted_send
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            id.to_owned(),
            Interrupted {
                turn,
                inputs,
                pending: true,
                terminal_event: false,
            },
        );
    Ok(())
}

/// The first RPC after resume. Reads use Vibe's real state, never the display projection above.
/// The caller's bounded open/reopen covers this entire wait and closes the channel on failure.
pub(crate) async fn stop_recovered(
    shared: &Shared,
    id: &str,
    resumed: &mut Value,
    model_hold: &mut Option<Box<dyn vw_supervisor::ModelRequestHold>>,
) -> Result<(), CoreError> {
    let Some(turn) = shared.interrupted_send.pending(id) else {
        return Ok(());
    };
    // The launcher may settle recovery before resume answers, before any terminal event is subscribed.
    // Retain that live, unprojected proof: a due loop can replace the old turn in the very next read.
    let terminal_on_resume = terminal_turn(&resumed["state"], &turn);
    let interrupted = shared
        .gate
        .call(
            "turn/interrupt",
            json!({"sessionId":id,"expectedTurnId":turn}),
            None,
        )
        .await
        .map_err(|e| CoreError::from_gate("turn/interrupt", &e));
    // Invalidate continuations of the old turn without publishing a separate user Stop action.
    crate::stops::stopped(shared, id);
    let mut confirmed = false;
    loop {
        let reply = shared
            .gate
            .call("session/read", json!({"sessionId":id}), None)
            .await
            .map_err(|e| CoreError::from_gate("session/read", &e))?;
        let state = &reply.result["state"];
        crate::resume_models::state(shared, id, state);
        let old_running = state["turns"].as_array().is_some_and(|turns| {
            turns
                .iter()
                .any(|t| t["id"] == turn && t["status"] == "in_progress")
        });
        if !confirmed
            && !old_running
            && (terminal_on_resume
                || terminal_turn(state, &turn)
                || shared.interrupted_send.terminal_event(id))
        {
            crate::resume_models::release(model_hold)?;
            confirmed = true;
        }
        let idle = confirmed && idle_state(state);
        if idle {
            crate::stops::finished(shared, &[id.to_owned()]);
            *resumed = reply.result;
            if let Some(old) = shared
                .interrupted_send
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_mut(id)
            {
                old.pending = false;
            }
            return Ok(());
        }
        // A stale expected-turn refusal is safe only with proof the old turn ended; a fresh loop may run.
        if let Err(error) = &interrupted
            && !confirmed
        {
            return Err(error.clone());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A read must describe the stopped turn, not just an idle session header.
#[cfg(test)]
fn stopped_state(state: &Value, turn: &str) -> bool {
    idle_state(state) && terminal_turn(state, turn)
}

fn idle_state(state: &Value) -> bool {
    matches!(
        state["session"]["status"]["type"].as_str(),
        Some("idle" | "failed")
    ) && state["turns"]
        .as_array()
        .is_some_and(|turns| turns.iter().all(|t| t["status"] != "in_progress"))
}

fn terminal_turn(state: &Value, turn: &str) -> bool {
    state["turns"].as_array().is_some_and(|turns| {
        turns.iter().any(|t| {
            t["id"] == turn
                && matches!(
                    t["status"].as_str(),
                    Some("completed" | "failed" | "interrupted")
                )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::stopped_state;
    use serde_json::json;

    #[test]
    fn interrupted_turn_must_be_present_and_terminal_before_fresh_work() {
        for turns in [
            json!([]),
            json!([{"id":"older","status":"completed"}]),
            json!([{"id":"old","status":"in_progress"}]),
            json!([{"id":"old"}]),
            json!([{"id":"old","status":"interrupted"},{"id":"other","status":"in_progress"}]),
        ] {
            assert!(
                !stopped_state(
                    &json!({"session":{"status":{"type":"idle"}},"turns":turns}),
                    "old"
                ),
                "{turns}"
            );
        }
        for status in ["interrupted", "completed", "failed"] {
            assert!(stopped_state(
                &json!({"session":{"status":{"type":"idle"}},"turns":[{"id":"old","status":status}]}),
                "old"
            ));
        }
        assert!(!stopped_state(
            &json!({"session":{"status":{"type":"busy"}},"turns":[{"id":"old","status":"interrupted"}]}),
            "old"
        ));
    }
}
