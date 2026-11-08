//! An explicit action or saved schedule stops an interrupted old turn before starting fresh work.
//! Uses ordinary gated resume/interrupt/read calls in the chat's own mode. Dispatch/recovery own the
//! chat lock; snapshots retain the old Interrupted row without inventing a Stop entry or hiding effects.
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use crate::core::{Core, Shared};
use crate::error::CoreError;
use crate::event::{CoreEvent, SessionChange};

struct Interrupted {
    turn: Value,
    pending: bool,
}

#[derive(Default)]
pub(crate) struct State(Mutex<HashMap<String, Interrupted>>);

impl State {
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
        return Ok(());
    };
    let mut turn = turn.clone();
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
                pending: true,
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
) -> Result<(), CoreError> {
    let Some(turn) = shared.interrupted_send.pending(id) else {
        return Ok(());
    };
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
    loop {
        let reply = shared
            .gate
            .call("session/read", json!({"sessionId":id}), None)
            .await
            .map_err(|e| CoreError::from_gate("session/read", &e))?;
        let state = &reply.result["state"];
        let idle = state["session"]["status"]["type"] == "idle"
            && state["turns"]
                .as_array()
                .is_some_and(|turns| turns.iter().all(|t| t["status"] != "in_progress"));
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
        // A stale expected-turn refusal is safe only after a read confirms there is no running turn.
        if let Err(error) = &interrupted {
            return Err(error.clone());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
