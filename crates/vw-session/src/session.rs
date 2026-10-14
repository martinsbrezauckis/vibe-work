//! One session's state of record plus the bookkeeping the store needs around it.
//!
//! Responsibility: own a session's wire-JSON `PublicSessionState`, its event watermark, resync and
//! local-only flags, anomaly log and a short log of recent events (for replay after a reload).
//! Also the state-level helpers shared by load and snapshot: suffix merge and interrupted-turn
//! marking. Event reduction lives in `reducer`.
//! Depends on: `anomaly`, `change`, `serde_json`. Called by: `store`, `reducer`.
//! Invariant: `last_event_id` is the watermark of `state`; events are applied strictly in
//! contiguous order, and nothing is applied while `resync_pending` is set.

use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::anomaly::{Anomalies, AnomalyKind};
use crate::change::{Change, Notification, SessionView};
use crate::patch::apply_patch;

/// How many recent event notifications each session keeps for replay after a `load`.
pub(crate) const EVENT_LOG_CAP: usize = 1024;

/// The owned state of one session. Fields are crate-private; outside code gets [`SessionView`].
#[derive(Debug)]
pub(crate) struct SessionEntry {
    pub(crate) state: Value,
    pub(crate) last_event_id: u64,
    pub(crate) local_only: bool,
    pub(crate) resync_pending: bool,
    pub(crate) interrupted_turns: Vec<String>,
    pub(crate) anomalies: Anomalies,
    pub(crate) runtime: Option<Value>,
    /// Recent event notifications in arrival order (applied or held back), newest last.
    pub(crate) log: VecDeque<Notification>,
}

/// Reads `state.eventId` as an unsigned integer (0 when absent: a fresh session has none).
fn watermark(state: &Value) -> u64 {
    state.get("eventId").and_then(Value::as_u64).unwrap_or(0)
}

impl SessionEntry {
    /// Adopts `state` as a brand-new entry.
    pub(crate) fn new(state: Value, local_only: bool) -> Self {
        Self {
            last_event_id: watermark(&state),
            state,
            local_only,
            resync_pending: false,
            interrupted_turns: Vec::new(),
            anomalies: Anomalies::default(),
            runtime: None,
            log: VecDeque::new(),
        }
    }

    /// Session id of the held state.
    pub(crate) fn id(&self) -> &str {
        self.state
            .pointer("/session/id")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// Copies the entry into a public view.
    pub(crate) fn view(&self) -> SessionView {
        SessionView {
            session_id: self.id().to_owned(),
            state: self.state.clone(),
            last_event_id: self.last_event_id,
            local_only: self.local_only,
            resync_pending: self.resync_pending,
            interrupted_turns: self.interrupted_turns.clone(),
            anomalies: self.anomalies.counts(),
        }
    }

    /// Remembers an event notification for later replay, dropping the oldest beyond the cap.
    pub(crate) fn remember(&mut self, n: Notification) {
        if self.log.len() == EVENT_LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(n);
    }

    /// Replaces the state wholesale (resync, resume, compaction in place) and clears the resync
    /// flag. Returns the logged events newer than the new watermark, sorted and de-duplicated,
    /// for the caller to replay. Mirrors `ClientProjection._replace_state`.
    ///
    /// WORKAROUND(vibe 2.26.0): after `session/resume` the server numbers the session's events again
    /// from the resumed state's `eventId`, so the new watermark can be LOWER than the old one. The
    /// logged events then belong to an older numbering; replaying those "newer" than the new
    /// watermark applied stale patches to the fresh state, failed, asked for a resync, and every
    /// resync read replayed them again (5,000 reads in seconds, seen live 2026-10-08). A watermark
    /// that goes backwards therefore discards the log. Remove when the server keeps one numbering
    /// per session.
    pub(crate) fn replace_state(&mut self, state: Value) -> Vec<Notification> {
        let previous = self.last_event_id;
        self.last_event_id = watermark(&state);
        if self.last_event_id < previous {
            self.log.clear();
        }
        self.state = state;
        self.resync_pending = false;
        let mark = self.last_event_id;
        let mut backlog: Vec<(u64, Notification)> = self
            .log
            .drain(..)
            .filter_map(|n| {
                let id = n.params.get("eventId").and_then(Value::as_u64)?;
                (id > mark).then_some((id, n))
            })
            .collect();
        backlog.sort_by_key(|(id, _)| *id);
        backlog.dedup_by_key(|(id, _)| *id);
        backlog.into_iter().map(|(_, n)| n).collect()
    }

    /// Copies the active model alias of the stored runtime snapshot into `session.model`.
    /// Returns the change when it differed. `reasoningEffort` is deliberately not folded: the
    /// legacy harness reports `null` while its runtime says `"off"`, so the value
    /// cannot be derived from the runtime reliably; readers take the effort from the runtime.
    pub(crate) fn fold_runtime_into_session(&mut self) -> Option<Change> {
        let alias = self
            .runtime
            .as_ref()?
            .pointer("/config/activeModel/alias")?
            .as_str()?
            .to_owned();
        let session = self.state.get_mut("session")?;
        if session.get("model").and_then(Value::as_str) == Some(alias.as_str()) {
            return None;
        }
        let patch = vec![json!({"op": "add", "path": "/model", "value": alias})];
        apply_patch(session, &patch).ok()?;
        Some(Change::SessionPatched {
            session_id: self.id().to_owned(),
            patch,
        })
    }

    /// Marks turns that were `in_progress` in `previous_turns` but are absent from the current
    /// state as `interrupted` (a server crash can lose the in-flight turn silently).
    /// A turn the server still reports in any status is left to the server. Returns the new
    /// turn objects so the caller can fan them out.
    pub(crate) fn mark_interrupted(&mut self, previous_turns: &[Value]) -> Vec<Value> {
        let mut marked = Vec::new();
        for old in previous_turns {
            let (Some(id), Some("in_progress")) = (
                old.get("id").and_then(Value::as_str),
                old.get("status").and_then(Value::as_str),
            ) else {
                continue;
            };
            let known = self
                .state
                .get("turns")
                .and_then(Value::as_array)
                .is_some_and(|t| {
                    t.iter()
                        .any(|x| x.get("id").and_then(Value::as_str) == Some(id))
                });
            if known {
                continue;
            }
            let mut turn = old.clone();
            turn["status"] = Value::from("interrupted");
            match self.state.get_mut("turns") {
                Some(Value::Array(turns)) => turns.push(turn.clone()),
                _ => self.state["turns"] = Value::Array(vec![turn.clone()]),
            }
            self.interrupted_turns.push(id.to_owned());
            self.anomalies.record(
                AnomalyKind::InterruptedTurn,
                None,
                format!("turn {id} was running before the resume and is gone; marked interrupted"),
            );
            marked.push(turn);
        }
        marked
    }
}

/// Ids of `items` (history entries or turns) in order; items without a string id yield `""`.
fn ids(items: &[Value]) -> Vec<&str> {
    items
        .iter()
        .map(|i| i.get("id").and_then(Value::as_str).unwrap_or_default())
        .collect()
}

/// Keeps an already-loaded prefix in front of a newer page `current` (a snapshot or read returns
/// only the latest page). Port of `events.py _merge_snapshot_suffix`. Returns the merged list and
/// whether a non-empty prefix was preserved.
pub(crate) fn merge_suffix(previous: &Value, current: &Value) -> (Value, bool) {
    let (Some(prev), Some(cur)) = (previous.as_array(), current.as_array()) else {
        return (
            if current.is_null() {
                previous.clone()
            } else {
                current.clone()
            },
            false,
        );
    };
    if prev.is_empty() || cur.is_empty() {
        return (current.clone(), false);
    }
    let first = cur[0].get("id").and_then(Value::as_str).unwrap_or_default();
    let Some(start) = ids(prev).iter().position(|id| *id == first) else {
        return (current.clone(), false);
    };
    let overlap = &prev[start..];
    if overlap.len() > cur.len() || ids(overlap) != ids(&cur[..overlap.len()]) {
        return (current.clone(), false);
    }
    let mut merged = prev[..start].to_vec();
    merged.extend(cur.iter().cloned());
    (Value::Array(merged), start > 0)
}

/// Builds the state to adopt when a same-session snapshot or read arrives: the new state with
/// the already-loaded history/turn prefix retained (`_apply_snapshot`).
pub(crate) fn merged_with_prefix(previous: &Value, mut next: Value) -> Value {
    let (history, kept) = merge_suffix(
        previous.get("history").unwrap_or(&Value::Null),
        next.get("history").unwrap_or(&Value::Null),
    );
    let (turns, _) = merge_suffix(
        previous.get("turns").unwrap_or(&Value::Null),
        next.get("turns").unwrap_or(&Value::Null),
    );
    if kept {
        next["historyBeforeCursor"] = previous
            .get("historyBeforeCursor")
            .cloned()
            .unwrap_or(Value::Null);
    }
    // A page requested without history carries `history: null` and an empty `activeCallbacks`
    // Keep what we know instead of closing callbacks that are still open.
    if next.get("history").is_none_or(Value::is_null) {
        next["activeCallbacks"] = previous
            .get("activeCallbacks")
            .cloned()
            .unwrap_or_else(|| json!([]));
    }
    next["history"] = history;
    next["turns"] = turns;
    next
}
