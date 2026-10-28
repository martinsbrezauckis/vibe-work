//! The event reducer: applies one session-scoped notification to a [`SessionEntry`].
//!
//! Responsibility: sequencing (duplicate / gap / resync-hold) and the per-notification rules.
//! Rules follow Vibe's
//! `ClientProjection` (`vibe/app_server/events.py`), with these deliberate differences:
//! - a re-announced entry id is upserted and counted, not an error;
//! - a gap is NOT applied through: the event is held back and a resync is requested, so the
//!   state of record never contains a hole;
//! - a failed patch leaves the state unchanged and requests a resync (Vibe raises).
//!
//! Depends on: `session`, `patch`, `anomaly`, `change`. Called by: `store`.

use serde_json::{Value, json};

use crate::anomaly::AnomalyKind;
use crate::change::{Action, Applied, Change, Notification};
use crate::patch::apply_patch;
use crate::session::{SessionEntry, merged_with_prefix};

/// Why a handler could not apply an event; both variants hold the event back and ask for a resync.
enum Failure {
    Patch(String),
    Malformed(String),
    Envelope(String),
}

type Handled = Result<Vec<Change>, Failure>;

impl SessionEntry {
    /// Applies one event-bearing notification (`params` has `eventId`) for this session.
    /// Never panics; irregularities become anomalies and, where state may be stale, an
    /// [`Action::NeedsResync`].
    pub(crate) fn apply_event(&mut self, n: Notification) -> Applied {
        let mut out = Applied::default();
        let Some(event_id) = n.params.get("eventId").and_then(Value::as_u64) else {
            self.anomalies.record(
                AnomalyKind::MalformedPayload,
                None,
                format!(
                    "{}: eventId missing or not a non-negative integer",
                    n.method
                ),
            );
            return out;
        };
        // Vibe 3's deferred copy setup emits ephemeral cwd events, but reads still return
        // watermark 0. Promotion on the first message starts durable numbering at 1 again,
        // with bumpedAt. Do not let an old setup gap hold that new stream indefinitely.
        // A session that never bumped reads `bumpedAt == createdAt` (Vibe 3.0.1+
        // `PublicSession._default_bumped_at`; 3.0.0 sent null).
        if event_id == 1
            && n.method == "session/updated"
            && self.state["session"]["harness"] == "unified"
            && self.state["session"]["bumpedAt"] == self.state["session"]["createdAt"]
            && self.state["turns"].as_array().is_some_and(Vec::is_empty)
            && self.state["history"].as_array().is_some_and(Vec::is_empty)
            && n.params["patch"].as_array().is_some_and(|patch| {
                patch.len() == 1
                    && patch[0]["op"] == "replace"
                    && patch[0]["path"] == "/bumpedAt"
                    && patch[0]["value"].is_number()
            })
        {
            self.last_event_id = 0;
            self.resync_pending = false;
            self.log.clear();
        }
        if event_id <= self.last_event_id {
            self.anomalies.record(
                AnomalyKind::DuplicateEvent,
                Some(event_id),
                format!(
                    "{} already covered by watermark {}",
                    n.method, self.last_event_id
                ),
            );
            return out;
        }
        if self.resync_pending {
            // Held back; `load` replays everything newer than the reloaded watermark.
            self.remember(n);
            return out;
        }
        let expected = self.last_event_id + 1;
        if event_id != expected {
            let reason = format!(
                "{}: event id gap, expected {expected}, got {event_id}",
                n.method
            );
            self.anomalies
                .record(AnomalyKind::EventGap, Some(event_id), reason.clone());
            self.hold_for_resync(n, &mut out, reason);
            return out;
        }
        match self.dispatch(&n) {
            Ok(changes) => {
                self.last_event_id = event_id;
                out.changes = changes;
                self.remember(n);
            }
            Err(failure) => {
                let (kind, reason) = match failure {
                    Failure::Patch(r) => (AnomalyKind::PatchFailed, r),
                    Failure::Malformed(r) => (AnomalyKind::MalformedPayload, r),
                    Failure::Envelope(r) => (AnomalyKind::InconsistentEnvelope, r),
                };
                let reason = format!("{} event {event_id}: {reason}", n.method);
                self.anomalies.record(kind, Some(event_id), reason.clone());
                self.hold_for_resync(n, &mut out, reason);
            }
        }
        out
    }

    fn hold_for_resync(&mut self, n: Notification, out: &mut Applied, reason: String) {
        self.resync_pending = true;
        out.actions.push(Action::NeedsResync {
            session_id: self.id().to_owned(),
            reason,
            delayed: false,
        });
        self.remember(n);
    }

    fn dispatch(&mut self, n: &Notification) -> Handled {
        let p = &n.params;
        let sid = self.id().to_owned();
        match n.method.as_str() {
            "session/snapshot" => self.on_snapshot(p, sid),
            "session/updated" => {
                let patch = array_field(p, "patch")?;
                guard_identity(&patch)?;
                let session = self
                    .state
                    .get_mut("session")
                    .ok_or_else(|| missing("state.session"))?;
                apply_patch(session, &patch).map_err(|e| Failure::Patch(e.to_string()))?;
                let mut changes = vec![Change::SessionPatched {
                    session_id: sid,
                    patch,
                }];
                changes.extend(self.fold_runtime_into_session());
                Ok(changes)
            }
            "history/entryAdded" => self.on_entry_added(p, &sid),
            "history/entryUpdated" => self.on_entry_updated(p, &sid),
            "turn/started" | "turn/completed" => {
                let turn = object_field(p, "turn")?;
                self.upsert_turn(turn.clone())?;
                Ok(vec![Change::TurnUpserted {
                    session_id: sid,
                    turn,
                }])
            }
            "turn/queueUpdated" => {
                let queue = object_field(p, "queue")?;
                self.state["turnQueue"] = queue.clone();
                Ok(vec![Change::QueueReplaced {
                    session_id: sid,
                    queue,
                }])
            }
            "session/statsUpdated" => self.on_stats(p, sid),
            "session/childSessionUpdated" => self.on_child(p, sid),
            // Unknown event-bearing methods: consume the id so the sequence stays contiguous.
            _ => Ok(Vec::new()),
        }
    }

    fn on_snapshot(&mut self, p: &Value, sid: String) -> Handled {
        let state = object_field(p, "state")?;
        let same_session = state.pointer("/session/id").and_then(Value::as_str)
            == Some(sid.as_str())
            && p.get("sessionId").and_then(Value::as_str) == Some(sid.as_str());
        let same_mark = state.get("eventId") == p.get("eventId");
        if !same_session || !same_mark {
            return Err(Failure::Envelope(
                "state does not match its notification envelope".into(),
            ));
        }
        self.state = merged_with_prefix(&self.state, state);
        let _ = self.fold_runtime_into_session();
        Ok(vec![Change::Replaced { session_id: sid }])
    }

    fn on_entry_added(&mut self, p: &Value, sid: &str) -> Handled {
        let entry = object_field(p, "entry")?;
        let id = str_field(&entry, "id")?.to_owned();
        let history = self.history_mut()?;
        let existing = history
            .iter()
            .position(|e| e.get("id").and_then(Value::as_str) == Some(&id));
        let replaced = existing.is_some();
        match existing {
            Some(i) => history[i] = entry.clone(),
            None => history.push(entry.clone()),
        }
        if replaced {
            self.anomalies.record(
                AnomalyKind::ReannouncedEntry,
                p.get("eventId").and_then(Value::as_u64),
                format!("history/entryAdded re-announced entry {id}; upserted"),
            );
        }
        let mut changes = vec![Change::EntryUpserted {
            session_id: sid.to_owned(),
            entry: entry.clone(),
            replaced,
        }];
        changes.extend(self.track_callback(&entry, sid));
        Ok(changes)
    }

    fn on_entry_updated(&mut self, p: &Value, sid: &str) -> Handled {
        let entry_id = str_field(p, "entryId")?.to_owned();
        let patch = array_field(p, "patch")?;
        guard_identity(&patch)?;
        let position = self
            .history_mut()?
            .iter()
            .rposition(|e| e.get("id").and_then(Value::as_str) == Some(&entry_id));
        let Some(index) = position else {
            return self.unknown_entry_update(p, entry_id);
        };
        let slot = &mut self.history_mut()?[index];
        let frozen = slot.get("generationStatus").and_then(Value::as_str) == Some("completed");
        apply_patch(slot, &patch).map_err(|e| Failure::Patch(e.to_string()))?;
        let updated = slot.clone();
        if frozen {
            self.anomalies.record(
                AnomalyKind::CompletedEntryUpdated,
                p.get("eventId").and_then(Value::as_u64),
                format!("completed entry {entry_id} was updated; patch applied"),
            );
        }
        let mut changes = vec![Change::EntryPatched {
            session_id: sid.to_owned(),
            entry_id,
            patch,
        }];
        changes.extend(self.track_callback(&updated, sid));
        Ok(changes)
    }

    /// An update for an entry the view never saw added. Vibe can turn the add of a streamed final answer into an
    /// update (upstream issue 1336); dropping it would lose the answer from the view, while the server's state has
    /// it. When the entry's turn is loaded, reload once for that entry; an entry of a turn outside the loaded window
    /// is ignored, as Vibe's own projection does, and so is a second update for an entry the reload still lacked.
    fn unknown_entry_update(&mut self, p: &Value, entry_id: String) -> Handled {
        let turn = p.get("turnId").and_then(Value::as_str);
        let loaded_turn = turn.is_some_and(|turn| {
            self.state["turns"]
                .as_array()
                .is_some_and(|turns| turns.iter().any(|t| t["id"] == turn))
        });
        if !loaded_turn || self.resynced_for_entry.as_deref() == Some(entry_id.as_str()) {
            return Ok(Vec::new());
        }
        let reason = format!("update for entry {entry_id} that was never added");
        self.resynced_for_entry = Some(entry_id);
        Err(Failure::Patch(reason))
    }

    fn on_stats(&mut self, p: &Value, sid: String) -> Handled {
        let stats = object_field(p, "stats")?;
        let num = |k: &str| stats.get(k).and_then(Value::as_u64);
        let (Some(input), Some(output)) =
            (num("sessionPromptTokens"), num("sessionCompletionTokens"))
        else {
            return Err(missing("stats.sessionPromptTokens/sessionCompletionTokens"));
        };
        // `AgentStatsSnapshot.token_usage`: total is sessionTotalLlmTokens, not input + output.
        let total = num("sessionTotalLlmTokens").unwrap_or(input + output);
        let mut usage = json!({"inputTokens": input, "outputTokens": output, "totalTokens": total});
        // Vibe 3.0.1+ (`AgentStatsSnapshot.token_usage`): the prompt-cache part of the input, as a read carries it.
        if let Some(cached) = num("sessionCachedTokens") {
            usage["cachedInputTokens"] = json!(cached);
        }
        let session = self
            .state
            .get_mut("session")
            .ok_or_else(|| missing("state.session"))?;
        session["tokenUsage"] = usage.clone();
        // Context fill: kept apart from `tokenUsage` and from Vibe's own `session.contextUsage`
        // (a token usage of the current context), which mirror Vibe's fields.
        let context_usage = match (
            num("contextTokens"),
            p.get("contextWindow").and_then(Value::as_u64),
        ) {
            (Some(tokens), Some(window)) => {
                let value = json!({"contextTokens": tokens, "contextWindow": window});
                session["contextFill"] = value.clone();
                Some(value)
            }
            _ => None,
        };
        Ok(vec![Change::TokenUsageSet {
            session_id: sid,
            token_usage: usage,
            context_usage,
        }])
    }

    fn on_child(&mut self, p: &Value, sid: String) -> Handled {
        let child = object_field(p, "childSession")?;
        let id = str_field(&child, "id")?.to_owned();
        if !self.state.get("childSessions").is_some_and(Value::is_array) {
            self.state["childSessions"] = json!([]);
        }
        let Some(children) = self.state["childSessions"].as_array_mut() else {
            return Err(missing("state.childSessions"));
        };
        if let Some(slot) = children
            .iter_mut()
            .find(|c| c.get("id").and_then(Value::as_str) == Some(&id))
        {
            *slot = child.clone();
        } else {
            children.push(child.clone());
            // Same order as Vibe's projection: (createdAt, id).
            children.sort_by(|a, b| {
                let key = |c: &Value| {
                    (
                        c.get("createdAt")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                        c.get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    )
                };
                key(a).cmp(&key(b))
            });
        }
        Ok(vec![Change::ChildUpserted {
            session_id: sid,
            child,
        }])
    }

    /// Upserts a turn by id (`turns` may be null before the first turn).
    fn upsert_turn(&mut self, turn: Value) -> Result<(), Failure> {
        let id = str_field(&turn, "id")?.to_owned();
        if !self.state.get("turns").is_some_and(Value::is_array) {
            self.state["turns"] = json!([]);
        }
        let Some(turns) = self.state["turns"].as_array_mut() else {
            return Err(missing("state.turns"));
        };
        match turns
            .iter_mut()
            .find(|t| t.get("id").and_then(Value::as_str) == Some(&id))
        {
            Some(slot) => *slot = turn,
            None => turns.push(turn),
        }
        Ok(())
    }

    /// `state.history` as a mutable vector, creating it when null (`PublicSessionState.history`
    /// is nullable).
    fn history_mut(&mut self) -> Result<&mut Vec<Value>, Failure> {
        if !self.state.get("history").is_some_and(Value::is_array) {
            self.state["history"] = json!([]);
        }
        self.state
            .get_mut("history")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| missing("state.history"))
    }

    /// Re-derives `activeCallbacks` membership for a callback entry (remove by `callbackId`, add
    /// back at the end when still open), as `ClientProjection._update_entry` does. Returns a
    /// `CallbacksChanged` when the open set changed.
    fn track_callback(&mut self, entry: &Value, sid: &str) -> Option<Change> {
        if entry.get("type").and_then(Value::as_str) != Some("callback") {
            return None;
        }
        let callback_id = entry.get("callbackId").cloned().unwrap_or(Value::Null);
        let open = entry.pointer("/state/status").and_then(Value::as_str) == Some("open");
        let before = self.open_callback_ids();
        let mut active: Vec<Value> = self
            .state
            .get("activeCallbacks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        active.retain(|c| c.get("callbackId") != Some(&callback_id));
        if open {
            active.push(entry.clone());
        }
        self.state["activeCallbacks"] = Value::Array(active);
        let after = self.open_callback_ids();
        (before != after).then(|| Change::CallbacksChanged {
            session_id: sid.to_owned(),
            open_callback_ids: after,
        })
    }

    fn open_callback_ids(&self) -> Vec<String> {
        self.state
            .get("activeCallbacks")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        c.get("callbackId")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Rejects patches that would replace the patched object itself or change its `id`: the store
/// keys sessions and entries by id, so such a patch would corrupt the state of record. Checked on
/// the operations (not by cloning the target), so streaming appends stay O(patch).
fn guard_identity(patch: &[Value]) -> Result<(), Failure> {
    let hits_identity =
        |op: &Value| matches!(op.get("path").and_then(Value::as_str), Some("" | "/id"));
    if patch.iter().any(hits_identity) {
        return Err(Failure::Patch(
            "patch replaces the object or changes its id".into(),
        ));
    }
    Ok(())
}

fn missing(what: &str) -> Failure {
    Failure::Malformed(format!("missing or wrong type: {what}"))
}

fn object_field(p: &Value, key: &str) -> Result<Value, Failure> {
    match p.get(key) {
        Some(v) if v.is_object() => Ok(v.clone()),
        _ => Err(missing(key)),
    }
}

fn array_field(p: &Value, key: &str) -> Result<Vec<Value>, Failure> {
    match p.get(key) {
        Some(Value::Array(a)) => Ok(a.clone()),
        _ => Err(missing(key)),
    }
}

fn str_field<'a>(p: &'a Value, key: &str) -> Result<&'a str, Failure> {
    p.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| missing(key))
}

#[cfg(test)]
mod promotion_tests {
    use super::*;
    use crate::{LoadKind, SessionStore};

    fn patch(id: u64, path: &str, value: &Value) -> Notification {
        Notification {
            method: "session/updated".to_owned(),
            params: json!({"sessionId":"copy", "eventId":id,
                "patch":[{"op":"replace", "path":path, "value":value}]}),
        }
    }

    /// A resync asked for after a short delay (not now); Debug text, so the test compiles against either shape.
    fn delayed(action: &Action) -> bool {
        format!("{action:?}").contains("delayed: true")
    }

    /// Review round 2: a resync read that lags twice at the same watermark drops the held event (the turn's final
    /// idle status); the ONE delayed re-read it asks for then catches up, so the chat ends idle.
    #[test]
    fn a_read_lagging_twice_asks_for_one_delayed_read_that_catches_up() {
        let store = SessionStore::new();
        let state = |event: u64, status: &str| {
            json!({"session":{"id":"s", "harness":"unified", "createdAt":1000, "bumpedAt":2000,
                "status":{"type":status}}, "eventId":event, "turns":[{"id":"t", "status":"in_progress"}],
                "history":[]})
        };
        let _ = store.load(state(5, "running"), LoadKind::New).unwrap();
        let mut idle = patch(7, "/status", &json!({"type":"idle"}));
        idle.params["sessionId"] = json!("s");
        let _ = store.apply(idle);
        let first = store.load(state(5, "running"), LoadKind::Read).unwrap();
        assert!(
            matches!(first.actions.as_slice(), [a] if !delayed(a)),
            "{:?}",
            first.actions
        );
        let second = store.load(state(5, "running"), LoadKind::Read).unwrap();
        assert!(
            matches!(second.actions.as_slice(), [a] if delayed(a)),
            "a second lagging read asks for one delayed read: {:?}",
            second.actions
        );
        assert_eq!(
            store.view("s").unwrap().state["session"]["status"]["type"],
            "running"
        );
        let caught_up = store.load(state(7, "idle"), LoadKind::Read).unwrap();
        assert!(caught_up.actions.is_empty(), "{:?}", caught_up.actions);
        let view = store.view("s").unwrap();
        assert!(!view.resync_pending);
        assert_eq!(view.state["session"]["status"]["type"], "idle");
    }

    #[test]
    fn first_durable_turn_leaves_a_deferred_copy_setup_gap() {
        let store = SessionStore::new();
        let state = json!({"session":{"id":"copy", "harness":"unified", "createdAt":1000,
            "bumpedAt":1000, "cwd":"/main", "status":{"type":"idle"}}, "eventId":0, "turns":[], "history":[]});
        let _ = store.load(state.clone(), LoadKind::New).unwrap();
        let _ = store.apply(patch(1, "/cwd", &json!("/copy")));
        let _ = store.load(state, LoadKind::Read).unwrap();
        assert!(matches!(
            store
                .apply(patch(2, "/cwd", &json!("/copy")))
                .actions
                .as_slice(),
            [Action::NeedsResync { .. }]
        ));
        assert!(store.view("copy").unwrap().resync_pending);
        let _ = store.apply(patch(1, "/bumpedAt", &json!(1234)));
        let _ = store.apply(patch(2, "/status", &json!({"type":"running"})));
        let running = store.view("copy").unwrap();
        assert!(!running.resync_pending);
        assert_eq!(running.last_event_id, 2);
        assert_eq!(running.state["session"]["status"]["type"], "running");
        let _ = store.apply(patch(3, "/status", &json!({"type":"idle"})));
        // A late duplicate cannot rewind the stream after its first bump.
        let _ = store.apply(patch(1, "/bumpedAt", &json!(1234)));
        let ended = store.view("copy").unwrap();
        assert_eq!(ended.last_event_id, 3);
        assert_eq!(ended.state["session"]["status"]["type"], "idle");
    }

    /// A copy start as real Vibe 3.0.2 numbers it: the copy setup's ephemeral events 1 and 2, with the window's
    /// copy wait reading the session in between (watermark 0). The gap event 2 is held and remembered; resync reads
    /// answer watermark 0 again, so replaying event 2 stops asking for reads after the second one (the session is
    /// never paused by the pump's resync budget: `ResyncPaused`).
    #[test]
    fn a_resync_read_behind_a_held_gap_event_does_not_ask_again() {
        let store = SessionStore::new();
        let state = json!({"session":{"id":"copy", "harness":"unified", "createdAt":1000,
            "bumpedAt":1000, "cwd":"/main", "status":{"type":"idle"}}, "eventId":0, "turns":[], "history":[]});
        let _ = store.load(state.clone(), LoadKind::New).unwrap();
        let _ = store.apply(patch(1, "/cwd", &json!("/copy")));
        let mut read = state;
        read["session"]["cwd"] = json!("/copy");
        let _ = store.load(read.clone(), LoadKind::Read).unwrap();
        let held = store.apply(patch(2, "/cwd", &json!("/copy")));
        assert!(matches!(
            held.actions.as_slice(),
            [Action::NeedsResync { .. }]
        ));
        // The first resync read cannot tell a lagging read from Vibe's setup numbering: it asks once more
        // (`a_lagging_resync_read_still_asks_again_until_it_catches_up`). The second answers the same watermark
        // for the same held event and settles the session; no later read asks again.
        let first = store.load(read.clone(), LoadKind::Read).unwrap();
        assert!(
            matches!(first.actions.as_slice(), [Action::NeedsResync { .. }]),
            "{:?}",
            first.actions
        );
        // The second answers the same watermark for the same held event: the event is dropped and ONE delayed
        // re-read is asked for (the safety net for a read that lags twice); that read and any later one settle.
        let second = store.load(read.clone(), LoadKind::Read).unwrap();
        assert!(
            matches!(second.actions.as_slice(), [a] if delayed(a)),
            "one delayed re-read: {:?}",
            second.actions
        );
        for _ in 0..3 {
            let reread = store.load(read.clone(), LoadKind::Read).unwrap();
            assert!(
                reread.actions.is_empty(),
                "the read must settle the session: {:?}",
                reread.actions
            );
        }
        let settled = store.view("copy").unwrap();
        assert!(!settled.resync_pending);
        assert_eq!(settled.state["session"]["cwd"], "/copy");
        // The first message's durable stream (numbered from 1 again) applies as before.
        let _ = store.apply(patch(1, "/bumpedAt", &json!(1234)));
        let _ = store.apply(patch(2, "/status", &json!({"type":"running"})));
        let running = store.view("copy").unwrap();
        assert!(!running.resync_pending);
        assert_eq!(running.last_event_id, 2);
        assert_eq!(running.state["session"]["status"]["type"], "running");
    }

    /// A resync read that lags behind the live stream (watermark 5 while event 7,
    /// the turn's last, already arrived and 6 was lost) must still lead to another read, so the chat ends idle
    /// once Vibe's read catches up, instead of staying "running" until some later event.
    #[test]
    fn a_lagging_resync_read_still_asks_again_until_it_catches_up() {
        let store = SessionStore::new();
        let state = |event: u64, status: &str| {
            json!({"session":{"id":"s", "harness":"unified", "createdAt":1000, "bumpedAt":2000,
                "status":{"type":status}}, "eventId":event, "turns":[{"id":"t", "status":"in_progress"}],
                "history":[]})
        };
        let _ = store.load(state(5, "running"), LoadKind::New).unwrap();
        let mut idle = patch(7, "/status", &json!({"type":"idle"}));
        idle.params["sessionId"] = json!("s");
        let held = store.apply(idle);
        assert!(matches!(
            held.actions.as_slice(),
            [Action::NeedsResync { .. }]
        ));
        let lagging = store.load(state(5, "running"), LoadKind::Read).unwrap();
        assert!(
            matches!(lagging.actions.as_slice(), [Action::NeedsResync { .. }]),
            "a read behind the held event must ask again: {:?}",
            lagging.actions
        );
        let caught_up = store.load(state(7, "idle"), LoadKind::Read).unwrap();
        assert!(caught_up.actions.is_empty(), "{:?}", caught_up.actions);
        let view = store.view("s").unwrap();
        assert!(!view.resync_pending);
        assert_eq!(view.last_event_id, 7);
        assert_eq!(view.state["session"]["status"]["type"], "idle");
    }

    #[test]
    fn a_started_session_still_holds_gaps_and_ignores_old_bumps() {
        let store = SessionStore::new();
        let _ = store
            .load(
                json!({"session":{"id":"copy","harness":"unified","createdAt":1000,"bumpedAt":1000,
            "status":{"type":"idle"}}, "eventId":2,
            "turns":[{"id":"existing","status":"completed"}], "history":[]}),
                LoadKind::New,
            )
            .unwrap();
        let _ = store.apply(patch(4, "/status", &json!({"type":"running"})));
        let _ = store.apply(patch(1, "/bumpedAt", &json!(1234)));
        let view = store.view("copy").unwrap();
        assert!(view.resync_pending);
        assert_eq!(view.last_event_id, 2);
    }
}
