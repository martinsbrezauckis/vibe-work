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
                Ok(vec![Change::SessionPatched {
                    session_id: sid,
                    patch,
                }])
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
        let Some(slot) = self
            .history_mut()?
            .iter_mut()
            .rev()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(&entry_id))
        else {
            // Entry outside the loaded window: Vibe's projection ignores it too.
            return Ok(Vec::new());
        };
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
        let usage = json!({"inputTokens": input, "outputTokens": output, "totalTokens": total});
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
