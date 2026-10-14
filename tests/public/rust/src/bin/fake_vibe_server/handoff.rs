//! - `session/history/clear`: answers `{state, sessionLog}` with a NEW session id (history emptied); the old
//!   session stays stored; the channel that held it now holds the new one; no notification is sent.
//! - `session/rewind` with `inplace: false`: answers the rewind result with a new id whose history ends before the
//!   rewound entry; same channel rule.
//! - A turn whose message contains `vw-test-handoff` hands off INSIDE the turn (Vibe's `ContextClearedEvent`):
//!   `session/contextCleared {sessionId: new, oldSessionId, eventId, state, sessionLog}` goes out on the channel,
//!   and the rest of the turn runs as the new id.
//!
//! The agent configuration (MCP servers and their credential) is not touched: Vibe derives the new session from
//! the old one's settings, so its MCP server keeps the old credential.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::meta::now_ms;
use crate::script::Script;

/// Methods this module answers.
pub const HANDLES: &[&str] = &["session/history/clear"];

/// The marker that makes a turn hand off inside itself.
pub const IN_TURN_MARKER: &str = "vw-test-handoff";

impl Script {
    /// Copies `old`'s state under a new id (history cut to `keep` entries, or emptied), stores it and moves the
    /// channel holding `old` to it. Returns the new id and its state.
    fn hand_off_state(&self, old: &str, keep: Option<usize>) -> Option<(String, Value)> {
        let mut w = self.world.lock().unwrap();
        let mut state = w.sessions.get(old)?.clone();
        w.next_id += 1;
        let new = format!("{old}-h{}", w.next_id);
        state["session"]["id"] = json!(new);
        let history = state["history"].as_array().cloned().unwrap_or_default();
        state["history"] = json!(
            history
                .into_iter()
                .take(keep.unwrap_or(0))
                .collect::<Vec<_>>()
        );
        if keep.is_none() {
            state["turns"] = json!([]);
        }
        state["activeCallbacks"] = json!([]);
        w.sessions.insert(new.clone(), state.clone());
        if let Some(conn) = w.owner(old) {
            w.attached.insert(conn, new.clone());
        }
        drop(w);
        self.persist_now();
        Some((new, state))
    }

    /// Answers one of [`HANDLES`].
    pub fn handoff_call(self: &Arc<Self>, method: &str, params: &Value) -> Value {
        let sid = params["sessionId"].as_str().unwrap_or_default();
        match (method, self.hand_off_state(sid, None)) {
            ("session/history/clear", Some((_, state))) => {
                json!({"state": state, "sessionLog": {}})
            }
            _ => json!({"__error": "not_found"}),
        }
    }

    /// `session/rewind` with `inplace: false`: the conversation up to (not including) `entryId` as a new session.
    pub fn rewind_into_new(&self, params: &Value) -> Value {
        let sid = params["sessionId"].as_str().unwrap_or_default();
        let entry = params["entryId"].as_str().unwrap_or_default();
        let keep = self.world.lock().unwrap().sessions.get(sid).and_then(|s| {
            s["history"]
                .as_array()
                .and_then(|h| h.iter().position(|e| e["id"] == entry))
        });
        let Some(keep) = keep else {
            return json!({"__error": "not_found"});
        };
        match self.hand_off_state(sid, Some(keep)) {
            Some((_, state)) => json!({"state": state, "message": "rewound", "restoreErrors": [],
                "restoredPaths": [], "sessionLog": {}}),
            None => json!({"__error": "not_found"}),
        }
    }

    /// Inside a turn: when its message carries [`IN_TURN_MARKER`], announces a context clear to a new id
    /// (`session/contextCleared` on the session's channel) and returns the id the turn continues as.
    pub(crate) fn maybe_hand_off_in_turn(
        &self,
        sid: &str,
        turn_id: &str,
        params: &Value,
    ) -> String {
        if !params["message"].to_string().contains(IN_TURN_MARKER) {
            return sid.to_owned();
        }
        let target = self.target_now(sid);
        let Some((new, mut state)) = self.hand_off_state(sid, Some(usize::MAX)) else {
            return sid.to_owned();
        };
        let event_id = state["eventId"].as_u64().unwrap_or(0) + 1;
        state["eventId"] = json!(event_id);
        {
            let mut w = self.world.lock().unwrap();
            w.sessions.insert(new.clone(), state.clone());
            w.running.remove(sid);
            w.running.insert(new.clone(), turn_id.to_owned());
        }
        if let Some(send) = target {
            send(
                json!({"jsonrpc": "2.0", "method": "session/contextCleared", "params": {
                "sessionId": new, "oldSessionId": sid, "eventId": event_id, "state": state,
                "sessionLog": {}, "emittedAt": now_ms(), "planFilePath": null}}),
            );
        }
        new
    }
}
