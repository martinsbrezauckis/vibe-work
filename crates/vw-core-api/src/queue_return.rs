//! A message queued behind a stopped or failed turn goes back to the composer; it is never sent.
//!
//! Vibe pauses a session's turn queue when a turn ends `interrupted` or `failed` while messages wait in it
//! (harness `_session.py`: `if status in {"failed","interrupted"}: self._turn_queue.pause()`, the same in 2.26.1,
//! 3.0.0 and 3.0.2), and a paused queue runs again on `session/turn/queue/resume`. After Stop the queued
//! message is not sent: it returns to the composer, and a failed turn is treated the same (a blind retry
//! often fails again). So when Vibe announces a paused queue that holds messages, the core removes each waiting item
//! (`session/turn/queue/remove`: Vibe can then never run it; removing the last item also ends the pause,
//! `_turn_queue.py` `_reset_pause_if_empty`) and hands the removed items to the window (`CoreEvent::QueueReturned`).
//!
//! Depends on: `channels` (the requests), `vw_session::Change`, `event`. Called by: `pump::after_apply` for every change.
//! Invariants: one hand-back runs per session at a time. Successful item ids stay in the owning core until
//! deletion: partial announcements can reach the pump after the worker finishes, and Vibe reports success when
//! removing an already gone item. Keeping the ids across pauses prevents a second event even then. Failed removals
//! remain retryable in Vibe's paused queue (listed, never run, and logged).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::warn;

use crate::core::Shared;

/// Successful item ids survive worker completion and empty announcements, until the session is deleted.
#[derive(Default)]
pub(crate) struct Returns(Mutex<State>);

#[derive(Default)]
struct State {
    running: HashSet<String>,
    returned: HashMap<String, HashSet<String>>,
}

impl Returns {
    fn begin(&self, session: &str, items: Vec<Value>) -> Vec<Value> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.running.contains(session) {
            return Vec::new();
        }
        let mut seen = state.returned.get(session).cloned().unwrap_or_default();
        let fresh: Vec<_> = items
            .into_iter()
            .filter(|item| {
                item["id"]
                    .as_str()
                    .is_some_and(|id| seen.insert(id.to_owned()))
            })
            .collect();
        if !fresh.is_empty() {
            state.running.insert(session.to_owned());
        }
        fresh
    }

    fn finish(&self, session: &str, returned: &[Value]) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .returned
            .entry(session.to_owned())
            .or_default()
            .extend(
                returned
                    .iter()
                    .filter_map(|item| item["id"].as_str().map(str::to_owned)),
            );
        state.running.remove(session);
    }

    pub(crate) fn forget(&self, session: &str) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.returned.remove(session);
    }
}

/// Takes the messages of a paused queue out of Vibe and hands them back (see the module docs).
pub(crate) fn observe(shared: &Arc<Shared>, change: &vw_session::Change) {
    let vw_session::Change::QueueReplaced { session_id, queue } = change else {
        return;
    };
    let items: Vec<Value> = queue["items"].as_array().cloned().unwrap_or_default();
    if queue["paused"] != true || items.is_empty() {
        return;
    }
    let items = shared.queue_returns.begin(session_id, items);
    if items.is_empty() {
        return;
    }
    let (shared, session_id) = (shared.clone(), session_id.clone());
    tokio::spawn(async move {
        let mut returned = Vec::new();
        for item in items {
            let Some(id) = item["id"].as_str() else {
                continue;
            };
            match shared
                .channels
                .request(
                    "session/turn/queue/remove",
                    json!({"sessionId": session_id, "queueItemId": id}),
                )
                .await
            {
                Ok(_) => returned.push(item),
                // A refused removal remains retryable; never claim that its item was returned.
                Err(error) => {
                    warn!(session_id, %error, "queue: a queued message could not be taken back");
                }
            }
        }
        shared.queue_returns.finish(&session_id, &returned);
        if !returned.is_empty() {
            shared.emit(crate::event::CoreEvent::QueueReturned {
                session_id,
                items: returned,
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delayed_partial_announcement_after_the_worker_finishes_is_not_returned_again() {
        let state = Returns::default();
        let items = vec![json!({"id":"first"}), json!({"id":"second"})];
        let claimed = state.begin("chat", items.clone());
        assert_eq!(claimed, items);
        assert_eq!(
            state.begin("chat", vec![items[1].clone()]),
            Vec::<Value>::new()
        );
        state.finish("chat", &claimed);
        // The RPC replies finished before the pump delivered Vibe's partial queue announcement.
        assert_eq!(
            state.begin("chat", vec![items[1].clone()]),
            Vec::<Value>::new()
        );
        let next = vec![items[1].clone(), json!({"id":"next-pause"})];
        assert_eq!(state.begin("chat", next), vec![json!({"id":"next-pause"})]);
        assert_eq!(state.begin("other-chat", items.clone()), items);
    }

    #[test]
    fn a_failed_removal_can_be_retried() {
        let state = Returns::default();
        let items = vec![json!({"id":"retry"})];
        assert_eq!(state.begin("chat", items.clone()), items);
        state.finish("chat", &[]);
        assert_eq!(state.begin("chat", items.clone()), items);
    }
}
