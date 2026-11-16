//! Browser protection follows the pump's turn events: a finished turn or a replacement snapshot with no
//! turn in progress ends the chat's browser turn; an in-progress turn keeps it active.
use std::sync::Arc;

use serde_json::Value;
use vw_session::Change;

use crate::core::Shared;

use super::Pump;

impl Pump {
    /// Keeps browser protection in step with turn events and replacement snapshots.
    pub(super) fn browser_changed(&mut self, shared: &Arc<Shared>, change: &Change) {
        if let Change::TurnUpserted { session_id, turn } = change
            && finished_turn(turn)
        {
            crate::chrome_control::end_turn(shared, session_id);
            crate::paused_tools_commands::turn_finished(shared, session_id);
        }
        if let Change::Replaced { session_id } = change
            && let Some(view) = shared.store.view(session_id)
            && finished_snapshot(&view.state)
        {
            crate::chrome_control::end_turn(shared, session_id);
            crate::paused_tools_commands::turn_finished(shared, session_id);
        }
        if let Change::Replaced { session_id } = change
            && let Some(hub) = &shared.preview
            && let Some(view) = shared.store.view(session_id)
        {
            if view.state["turns"]
                .as_array()
                .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "in_progress"))
            {
                hub.service.browser_turn_active(session_id);
            } else if finished_snapshot(&view.state)
                && let Some(epoch) = hub.service.browser_turn_epoch(session_id)
            {
                let (shared, session_id) = (shared.clone(), session_id.clone());
                self.browser_releases.spawn(async move {
                    crate::browser::turn_ended(&shared, &session_id, epoch).await;
                });
            }
            hub.service.browser_turn_waiting(
                session_id,
                view.state["activeCallbacks"]
                    .as_array()
                    .is_some_and(|callbacks| !callbacks.is_empty()),
            );
        }
        if let Change::TurnUpserted { session_id, turn } = change
            && turn["status"] == "in_progress"
            && let Some(hub) = &shared.preview
        {
            hub.service.browser_turn_active(session_id);
        }
        if let Change::TurnUpserted { session_id, turn } = change
            && finished_turn(turn)
            && let Some(epoch) = shared
                .preview
                .as_ref()
                .and_then(|hub| hub.service.browser_turn_epoch(session_id))
        {
            let (shared, session_id) = (shared.clone(), session_id.clone());
            self.browser_releases.spawn(async move {
                crate::browser::turn_ended(&shared, &session_id, epoch).await;
            });
        }
    }
}

fn finished_turn(turn: &Value) -> bool {
    matches!(
        turn["status"].as_str(),
        Some("completed" | "failed" | "interrupted")
    )
}

fn finished_snapshot(state: &Value) -> bool {
    // A queued session/start snapshot has no completed turn. If it arrives after a browser action,
    // treating mere idleness as completion would release that newer work in every browser.
    state["turns"].as_array().is_some_and(|turns| {
        turns.iter().any(finished_turn) && !turns.iter().any(|turn| turn["status"] == "in_progress")
    })
}
