//! Session lifecycle on the store: loading results, handoff, removal and list reconciliation.
//!
//! Responsibility: everything that replaces or creates a session's state from a request result
//! (`session/start|resume|read|fork|rewind|compact`, `session/compacted` and `contextCleared`
//! notifications) and the local-only session rules. After a load it replays the
//! events that arrived around the response, so notifications overtaking a result are not lost.
//! Depends on: `store`, `session`, `reducer`. Called by: the transport pump after each request.

use serde_json::Value;

use crate::anomaly::AnomalyKind;
use crate::change::{Action, Applied, Change, Notification};
use crate::session::{SessionEntry, merged_with_prefix};
use crate::store::{Inner, SessionStore};

/// What produced the state being loaded; decides how it is merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadKind {
    /// `session/start` or `session/fork`: a session we just created. It is local-only until the
    /// server's `session/list` shows it.
    New,
    /// `session/resume` or `session/continue`: after a restart. A turn that was running before and
    /// is missing from the resumed state is marked interrupted locally.
    Resume,
    /// `session/read`: the latest page. An already-loaded history/turn prefix is kept in front of
    /// it, and entries in the page replace ours (so they win, e.g. `truncated` on effect output).
    Read,
    /// `session/rewind`, `session/compact`, `session/history/clear`: the state is replaced as is.
    Replace,
}

/// Why a state could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// `state.session.id` is missing, so the session cannot be keyed.
    #[error("state has no session.id")]
    MissingSessionId,
}

/// One row of the merged session list.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    /// The `PublicSession` object: our live copy when the session is loaded, else the server row.
    pub session: Value,
    /// True when the server's listing does not contain it yet.
    pub local_only: bool,
}

impl SessionStore {
    /// Adopts the `state` from a request result. The session id comes from the state itself.
    ///
    /// # Errors
    /// [`LoadError::MissingSessionId`] when the state has no `session.id`.
    pub fn load(&self, state: Value, kind: LoadKind) -> Result<Applied, LoadError> {
        self.with(|inner| inner.load(state, kind))
    }

    /// Marks the listed turns of `session_id` `interrupted`, whatever status they have now, and
    /// publishes them as `TurnUpserted`. For turns the client knows were cut off although the server
    /// reports them otherwise (Vibe 2.26.0 reports a turn killed by a server crash as `completed`;
    /// the caller decides which). Unknown sessions or turn ids are skipped.
    pub fn mark_interrupted(&self, session_id: &str, turn_ids: &[String]) -> Applied {
        self.with(|inner| {
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return Applied::default();
            };
            let mut marked = Vec::new();
            if let Some(Value::Array(turns)) = entry.state.get_mut("turns") {
                for turn in turns.iter_mut() {
                    let Some(id) = turn.get("id").and_then(Value::as_str).map(str::to_owned) else {
                        continue;
                    };
                    if !turn_ids.contains(&id) || turn["status"] == "interrupted" {
                        continue;
                    }
                    turn["status"] = Value::from("interrupted");
                    marked.push(turn.clone());
                    entry.interrupted_turns.push(id.clone());
                    entry.anomalies.record(
                        AnomalyKind::InterruptedTurn,
                        None,
                        format!("turn {id} was cut off by a server restart; marked interrupted"),
                    );
                }
            }
            let changes = marked
                .into_iter()
                .map(|turn| Change::TurnUpserted {
                    session_id: session_id.to_owned(),
                    turn,
                })
                .collect();
            inner.emit_all(changes)
        })
    }

    /// Drops a session (deleted or closed), e.g. after `session/delete`.
    pub fn remove(&self, session_id: &str) -> Applied {
        self.with(|inner| {
            inner.pending.remove(session_id);
            if inner.sessions.remove(session_id).is_some() {
                inner.emit_all(vec![Change::Removed {
                    session_id: session_id.to_owned(),
                }])
            } else {
                Applied::default()
            }
        })
    }

    /// Merges a `session/list` page (`items`) with sessions the server does not list yet.
    ///
    /// Sessions that now appear lose their local-only flag. Local-only sessions come first
    /// (newest first), then the server rows in the order given.
    #[must_use]
    pub fn reconcile_listing(&self, items: &[Value]) -> Vec<SessionRow> {
        self.with(|inner| {
            let mut rows = Vec::with_capacity(items.len());
            let mut listed = std::collections::BTreeSet::new();
            for item in items {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                listed.insert(id.to_owned());
                let session = match inner.sessions.get_mut(id) {
                    Some(entry) => {
                        entry.local_only = false;
                        entry
                            .state
                            .get("session")
                            .cloned()
                            .unwrap_or_else(|| item.clone())
                    }
                    None => item.clone(),
                };
                rows.push(SessionRow {
                    session,
                    local_only: false,
                });
            }
            let mut local: Vec<SessionRow> = inner
                .sessions
                .iter()
                .filter(|(id, e)| e.local_only && !listed.contains(*id))
                .filter_map(|(_, e)| e.state.get("session").cloned())
                .map(|session| SessionRow {
                    session,
                    local_only: true,
                })
                .collect();
            local.sort_by_key(|r| {
                std::cmp::Reverse(
                    r.session
                        .get("createdAt")
                        .and_then(Value::as_i64)
                        .unwrap_or_default(),
                )
            });
            local.extend(rows);
            local
        })
    }
}

fn session_id_of(state: &Value) -> Option<&str> {
    state.pointer("/session/id").and_then(Value::as_str)
}

impl Inner {
    pub(crate) fn load(&mut self, state: Value, kind: LoadKind) -> Result<Applied, LoadError> {
        let sid = session_id_of(&state)
            .ok_or(LoadError::MissingSessionId)?
            .to_owned();
        let (mut replay, interrupted) = self.adopt(&sid, state, kind);
        let mut out = self.emit_all(vec![Change::Replaced {
            session_id: sid.clone(),
        }]);
        let marked: Vec<Change> = interrupted
            .into_iter()
            .map(|turn| Change::TurnUpserted {
                session_id: sid.clone(),
                turn,
            })
            .collect();
        out.changes.extend(self.emit_all(marked).changes);
        replay.sort_by_key(|n| n.params.get("eventId").and_then(Value::as_u64));
        replay.dedup_by_key(|n| n.params.get("eventId").and_then(Value::as_u64));
        for n in replay {
            if let Some(entry) = self.sessions.get_mut(&sid) {
                let applied = entry.apply_event(n);
                out.changes.extend(self.emit_all(applied.changes).changes);
                out.actions.extend(applied.actions);
            }
        }
        Ok(out)
    }

    /// Installs `state` for `sid` (new entry or wholesale replacement). Returns the buffered or
    /// logged events to replay and the turns marked interrupted.
    fn adopt(
        &mut self,
        sid: &str,
        state: Value,
        kind: LoadKind,
    ) -> (Vec<Notification>, Vec<Value>) {
        let mut replay = self.take_pending(sid);
        let Some(entry) = self.sessions.get_mut(sid) else {
            let entry = SessionEntry::new(state, kind == LoadKind::New);
            let mark = entry.last_event_id;
            replay.retain(|n| n.params.get("eventId").and_then(Value::as_u64) > Some(mark));
            self.sessions.insert(sid.to_owned(), entry);
            return (replay, Vec::new());
        };
        let previous_turns = entry
            .state
            .get("turns")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let next = if kind == LoadKind::Read {
            merged_with_prefix(&entry.state, state)
        } else {
            state
        };
        replay.extend(entry.replace_state(next));
        let interrupted = if kind == LoadKind::Resume {
            entry.mark_interrupted(&previous_turns)
        } else {
            Vec::new()
        };
        (replay, interrupted)
    }

    /// `session/compacted` / `session/contextCleared`: the conversation moves to a new session id.
    pub(crate) fn handoff(&mut self, n: &Notification) -> Applied {
        let p = &n.params;
        let new_id = p
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let old_id = p
            .get("oldSessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let state = p.get("state").filter(|s| s.is_object()).cloned();
        let valid = match (&new_id, &old_id, &state) {
            (Some(new), Some(old), Some(state)) => {
                new != old
                    && session_id_of(state) == Some(new.as_str())
                    && state.get("eventId") == p.get("eventId")
                    && p.get("eventId")
                        .and_then(Value::as_u64)
                        .is_some_and(|e| e > 0)
            }
            _ => false,
        };
        let (Some(new_id), Some(state), true) = (new_id.clone(), state, valid) else {
            // Cannot trust the payload: note it on the old session and ask for a re-read.
            let mut out = Applied::default();
            if let Some(old) = old_id.as_deref().and_then(|o| self.sessions.get_mut(o)) {
                old.anomalies.record(
                    AnomalyKind::InconsistentEnvelope,
                    None,
                    format!("{}: invalid handoff payload", n.method),
                );
            }
            if let Some(target) = new_id.or(old_id) {
                out.actions.push(Action::NeedsResync {
                    session_id: target,
                    reason: format!("{}: invalid handoff payload", n.method),
                });
            }
            return out;
        };
        let old = old_id.and_then(|o| self.sessions.remove(&o).map(|e| (o, e)));
        let (mut out, carried) = match old {
            Some((old_id, e)) => (
                self.emit_all(vec![Change::HandedOff {
                    old_session_id: old_id,
                    new_session_id: new_id.clone(),
                }]),
                Some(e.anomalies),
            ),
            None => (Applied::default(), None),
        };
        match self.load(state, crate::lifecycle::LoadKind::Replace) {
            Ok(loaded) => {
                out.changes.extend(loaded.changes);
                out.actions.extend(loaded.actions);
            }
            Err(_) => return out,
        }
        if let (Some(carried), Some(entry)) = (carried, self.sessions.get_mut(&new_id)) {
            entry.anomalies = carried;
        }
        out
    }
}
