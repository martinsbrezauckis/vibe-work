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
    /// The last history entry an update arrived for before its add (`reducer::on_entry_updated`); one resync per
    /// entry, so an entry the reloaded state still lacks cannot start a resync per streamed update.
    pub(crate) resynced_for_entry: Option<String>,
    /// The last read that found a hole before a held event: (its watermark, the first event past the hole). A
    /// second read with the same pair cannot help (see [`Self::replace_state`]).
    pub(crate) hole_read: Option<(u64, u64)>,
    /// Set when a read dropped held events past a hole: the load asks for ONE delayed re-read.
    pub(crate) reread_later: bool,
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
            resynced_for_entry: None,
            hole_read: None,
            reread_later: false,
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
    /// Event ids are compared within one numbering only. After `session/resume` the session's
    /// events are numbered from the resumed state's `eventId`, so the new watermark can be LOWER
    /// than the old one. The logged events then belong to an older numbering, and replaying them
    /// would apply stale patches to the fresh state (and ask for a resync again and again), so a
    /// watermark that goes backwards discards the log.
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
        // A held event past a hole (an event between the watermark and it never arrived) re-opens the gap when
        // replayed, which asks for another read: right while Vibe's read is merely behind the live stream (the
        // next read catches up). Vibe 3's deferred copy setup numbers events its reads do not reflect (reads stay
        // at watermark 0 while event 2 was seen), and re-opening the same gap on every read would spend the resync
        // budget until the session paused (`ResyncPaused`). So: the first read
        // that finds a given hole replays as before; a second read that answers the same watermark for the same
        // held event cannot help, and only the run continuing the watermark is replayed (the rest is dropped;
        // the next live event continues the stream or asks for a fresh read).
        let contiguous = backlog
            .iter()
            .zip(mark + 1..)
            .take_while(|((id, _), want)| id == want)
            .count();
        let hole = backlog.get(contiguous).map(|(id, _)| (mark, *id));
        if hole.is_some() && hole == self.hole_read {
            backlog.truncate(contiguous);
            self.hole_read = None;
            // Safety net: a read that lagged twice catches up on one later read; Vibe's copy setup reads 0 again
            // and settles with nothing held.
            self.reread_later = true;
        } else {
            self.hole_read = hole;
        }
        backlog.into_iter().map(|(_, n)| n).collect()
    }

    /// Folds the active runtime model, and for unified sessions the thinking setting and the
    /// active agent, into the session.
    /// Legacy sessions keep their native effort: their null does not mean thinking is off.
    pub(crate) fn fold_runtime_into_session(&mut self) -> Option<Change> {
        let runtime = self.runtime.as_ref()?;
        let session = self.state.get_mut("session")?;
        let patch = runtime_patch(session, runtime);
        if patch.is_empty() {
            return None;
        }
        apply_patch(session, &patch).ok()?;
        Some(Change::SessionPatched {
            session_id: self.id().to_owned(),
            patch,
        })
    }

    /// Marks turns that were `in_progress` in `previous_turns` but are absent from the current
    /// state as `interrupted`, so a turn the user saw running never just disappears.
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
    retain_observations(previous, &mut next);
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

/// Runtime values omitted from session events by the unified server.
///
/// `agent`: a unified read carries the profile the chat runs on (Vibe 3.0.1+), while Vibe's turn-start
/// `session/updated` replaces it with null and nothing sets it again; Vibe's own client takes it from the
/// runtime's `activeAgent` (`client_state.py apply_runtime`). Legacy sessions keep their own reads' value.
fn runtime_patch(session: &Value, runtime: &Value) -> Vec<Value> {
    let mut patch = Vec::new();
    for (key, path) in [
        ("model", "/config/activeModel/alias"),
        ("reasoningEffort", "/config/activeModel/thinking"),
        ("agent", "/activeAgent"),
    ] {
        if key != "model" && session["harness"] != "unified" {
            continue;
        }
        let wanted = |v: &&Value| {
            if key == "agent" {
                v.is_object()
            } else {
                v.is_string()
            }
        };
        if let Some(value) = runtime.pointer(path).filter(wanted)
            && session.get(key) != Some(value)
        {
            patch.push(json!({"op":"add", "path":format!("/{key}"), "value":value}));
        }
    }
    patch
}

/// Keeps observations that native read pages omit, without replacing their authoritative entries.
fn retain_observations(previous: &Value, next: &mut Value) {
    if next["session"].get("contextFill").is_none()
        && let Some(fill) = previous["session"].get("contextFill")
    {
        next["session"]["contextFill"] = fill.clone();
    }
    if next["session"]["workspaceRoots"]
        .as_array()
        .is_some_and(Vec::is_empty)
        && let Some(roots) = previous["session"]["workspaceRoots"]
            .as_array()
            .filter(|r| !r.is_empty())
    {
        next["session"]["workspaceRoots"] = Value::Array(roots.clone());
    }
    let (Some(before), Some(history)) = (
        previous["history"].as_array(),
        next["history"].as_array_mut(),
    ) else {
        return;
    };
    // Closed approvals are transient in native reads. Retain them at their observed position next to an
    // entry the read still contains; never resurrect an entry after a rewind/replacement.
    for (at, entry) in before.iter().enumerate() {
        if entry["type"] != "callback"
            || entry.pointer("/state/status").and_then(Value::as_str) == Some("open")
            || history.iter().any(|e| e["id"] == entry["id"])
        {
            continue;
        }
        if let Some(after) = before[at + 1..]
            .iter()
            .find_map(|known| history.iter().position(|e| e["id"] == known["id"]))
        {
            history.insert(after, entry.clone());
        } else if let Some(prior) = before[..at]
            .iter()
            .rev()
            .find_map(|known| history.iter().position(|e| e["id"] == known["id"]))
        {
            history.insert(prior + 1, entry.clone());
        }
    }
}
