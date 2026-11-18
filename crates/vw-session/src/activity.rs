//! Small activity projection for idle-sleep decisions; reads the store without cloning histories.
//! Depends on wire state fields; called by the core's keep-awake reconciliation and its running-work check (Quit).

use crate::SessionStore;
use serde_json::Value;

/// Work and waiting facts, independent of the session's history size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Activity {
    child_running: bool,
    parent_working: bool,
    waiting: bool,
}

impl Activity {
    /// Projects only status, turn, child and callback facts from a wire state.
    #[must_use]
    pub fn from_state(state: &Value) -> Self {
        let status = |value: &Value| {
            value
                .get("type")
                .unwrap_or(value)
                .as_str()
                .unwrap_or_default()
                .to_owned()
        };
        let parent = status(&state["session"]["status"]);
        Self {
            child_running: state["childSessions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|child| status(&child["status"]) == "running"),
            parent_working: parent == "running"
                || (!matches!(parent.as_str(), "failed" | "archived")
                    && state["turns"].as_array().is_some_and(|turns| {
                        turns.iter().any(|turn| turn["status"] == "in_progress")
                    })),
            waiting: parent == "blocked"
                || state["activeCallbacks"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty()),
        }
    }

    /// A running child keeps working even when its parent waits for a callback.
    #[must_use]
    pub fn working(self, callback_pending: bool) -> bool {
        self.child_running || (self.parent_working && !self.waiting && !callback_pending)
    }

    /// Agent work runs in this chat: a turn in progress (also while it waits for the person's answer) or a running
    /// subagent. Quitting or restarting the app would end it.
    #[must_use]
    pub fn running(self) -> bool {
        self.child_running || self.parent_working
    }
}

impl SessionStore {
    /// Reads work/wait facts under the store lock without copying history or turn payloads.
    #[must_use]
    pub fn activity(&self, session_id: &str) -> Option<Activity> {
        self.lock()
            .sessions
            .get(session_id)
            .map(|entry| Activity::from_state(&entry.state))
    }
}

#[cfg(test)]
mod tests {
    //! Whether agent work runs, for the question before Quit and Restart: a turn in progress counts also while it
    //! waits for the person's answer (quitting would end it); a finished chat does not.
    //! PREDICT (before the change): there is no `running`; this does not compile.
    use super::Activity;
    use serde_json::json;

    #[test]
    fn a_turn_in_progress_runs_also_while_it_waits_for_an_answer() {
        let working = json!({"session": {"status": {"type": "running"}}, "turns": [{"status": "in_progress"}]});
        let waiting = json!({"session": {"status": {"type": "blocked"}}, "turns": [{"status": "in_progress"}],
            "activeCallbacks": [{"id": "c1"}]});
        let child = json!({"session": {"status": {"type": "idle"}}, "childSessions": [{"status": "running"}]});
        for state in [working, waiting, child] {
            assert!(Activity::from_state(&state).running(), "{state}");
        }
    }

    #[test]
    fn a_finished_or_failed_chat_does_not_run() {
        let idle =
            json!({"session": {"status": {"type": "idle"}}, "turns": [{"status": "completed"}]});
        let failed = json!({"session": {"status": {"type": "failed"}}, "turns": [{"status": "in_progress"}]});
        for state in [idle, failed, json!({})] {
            assert!(!Activity::from_state(&state).running(), "{state}");
        }
    }
}
