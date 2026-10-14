//! `SessionStore`: the shared handle through which the Rust core reads and updates session state.
//!
//! Responsibility: route notifications to the right session (buffering events that arrive before
//! their session is loaded), publish every resulting change to subscribers, and expose read views.
//! Load, handoff, local-only and listing logic is in `lifecycle`; per-event rules in `reducer`.
//! Depends on: `session`, `fanout`, `change`. Called by: the transport pump (writes), `vw-gate`,
//! the UI bridge and Debug API (reads / subscriptions).
//! Concurrency: all state sits behind one mutex; every method holds it only for in-memory work
//! (no I/O, no waiting on subscribers), so `apply` can never block on a slow consumer.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use serde_json::Value;

use crate::anomaly::Anomalies;
use crate::change::{Applied, Change, Notification, SessionView};
use crate::fanout::{Delivery, SubShared, Subscription};
use crate::session::{EVENT_LOG_CAP, SessionEntry};

/// Most sessions whose early events are buffered while the session itself is not loaded yet.
const MAX_PENDING_SESSIONS: usize = 32;

/// Shared, cheaply clonable store handle.
#[derive(Clone, Default)]
pub struct SessionStore {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
pub(crate) struct Inner {
    pub(crate) sessions: BTreeMap<String, SessionEntry>,
    /// Events for sessions not loaded yet (the notification can overtake the `session/start`
    /// response), replayed by `load`. Bounded per session and in session count.
    pub(crate) pending: BTreeMap<String, VecDeque<Notification>>,
    pending_order: VecDeque<String>,
    subs: Vec<Weak<SubShared>>,
    /// Subscribers that received something during the current call, to wake after unlock.
    touched: Vec<Arc<SubShared>>,
}

impl SessionStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `f` under the lock, then wakes touched subscribers after the lock is released.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let (result, touched) = {
            let mut g = self.lock();
            let r = f(&mut g);
            (r, std::mem::take(&mut g.touched))
        };
        for sub in touched {
            sub.wake();
        }
        result
    }

    /// Feeds one server notification. Session-scoped events update the state of record;
    /// `runtime/updated`, `turn/retrying`, warnings and auth notices become [`Change`]s only.
    /// Unknown methods without an `eventId` are ignored (the protocol may grow).
    #[must_use = "actions in the result must be carried out"]
    pub fn apply(&self, n: Notification) -> Applied {
        self.with(|inner| inner.apply(n))
    }

    /// Copy of one session's state and bookkeeping.
    #[must_use]
    pub fn view(&self, session_id: &str) -> Option<SessionView> {
        self.lock().sessions.get(session_id).map(SessionEntry::view)
    }

    /// Copies of every session, ordered by id.
    #[must_use]
    pub fn views(&self) -> Vec<SessionView> {
        self.lock()
            .sessions
            .values()
            .map(SessionEntry::view)
            .collect()
    }

    /// Every loaded session's id with its status type (`idle`, `running`, `blocked`,...), without
    /// copying any state: cheap enough to ask every few seconds.
    #[must_use]
    pub fn statuses(&self) -> Vec<(String, String)> {
        self.lock()
            .sessions
            .iter()
            .map(|(id, entry)| {
                let status = entry
                    .state
                    .pointer("/session/status")
                    .map(|s| match s {
                        Value::String(text) => text.clone(),
                        other => other
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    })
                    .unwrap_or_default();
                (id.clone(), status)
            })
            .collect()
    }

    /// The anomaly counters and recent records of one session (the Debug API's anomaly route).
    #[must_use]
    pub fn anomalies(&self, session_id: &str) -> Option<Anomalies> {
        self.lock()
            .sessions
            .get(session_id)
            .map(|e| e.anomalies.clone())
    }

    /// Latest `runtime/updated` snapshot seen for a session.
    #[must_use]
    pub fn runtime(&self, session_id: &str) -> Option<Value> {
        self.lock().sessions.get(session_id)?.runtime.clone()
    }

    /// Subscribes: the first items are a snapshot per session, then ordered changes. At most
    /// `capacity` undelivered items are kept; beyond that the subscriber is resnapshotted.
    #[must_use]
    pub fn subscribe(&self, capacity: usize) -> Subscription {
        let shared = Arc::new(SubShared::new(capacity));
        {
            let mut g = self.lock();
            shared.push_initial(g.all_views());
            g.subs.push(Arc::downgrade(&shared));
        }
        Subscription {
            shared,
            store: self.clone(),
        }
    }

    /// Refills a lagged subscriber with snapshots of every session (see `fanout`).
    pub(crate) fn rebuild_subscriber(&self, shared: &Arc<SubShared>) {
        let g = self.lock();
        shared.rebuild(g.all_views());
    }
}

impl Inner {
    pub(crate) fn all_views(&self) -> Vec<SessionView> {
        self.sessions.values().map(SessionEntry::view).collect()
    }

    pub(crate) fn apply(&mut self, n: Notification) -> Applied {
        match n.method.as_str() {
            "session/compacted" | "session/contextCleared" => self.handoff(&n),
            "runtime/updated" => self.runtime_updated(&n),
            _ if n.params.get("eventId").is_some() && n.params.get("sessionId").is_some() => {
                self.session_event(n)
            }
            "turn/retrying"
            | "warning"
            | "error"
            | "mcp_catalog/authRequired"
            | "connector_catalog/authRequired"
            // the sign-in page address of a connector or server, and a connector sign-in Vibe
            // could not start. Not secrets (state + PKCE challenge); the UI shows them in the link dialog.
            | "connector_catalog/authUrl"
            | "connector_catalog/authFailed"
            | "mcp_catalog/authUrl" => {
                let session_id = n
                    .params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.emit_all(vec![Change::Notice {
                    session_id,
                    method: n.method,
                    params: n.params,
                }])
            }
            _ => Applied::default(),
        }
    }

    /// `runtime/updated`: remember the snapshot and fold its active model into the session.
    ///
    /// The server fills `session.model` and `session.reasoningEffort` from the runtime on its
    /// own, without a `session/updated` event (a read after the first turn can show
    /// `model: "cli-fast"` while the start result and every event had `null`). Vibe's own client
    /// does the same fold (`client_state.py apply_runtime`: `session.model = active_model.alias`).
    fn runtime_updated(&mut self, n: &Notification) -> Applied {
        let (Some(sid), Some(runtime)) = (
            n.params.get("sessionId").and_then(Value::as_str),
            n.params.get("runtime"),
        ) else {
            return Applied::default();
        };
        let (sid, runtime) = (sid.to_owned(), runtime.clone());
        let mut changes = Vec::new();
        if let Some(entry) = self.sessions.get_mut(&sid) {
            entry.runtime = Some(runtime.clone());
            changes.extend(entry.fold_runtime_into_session());
        }
        changes.push(Change::RuntimeUpdated {
            session_id: sid,
            runtime,
        });
        self.emit_all(changes)
    }

    fn session_event(&mut self, n: Notification) -> Applied {
        let Some(sid) = n
            .params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Applied::default();
        };
        let Some(entry) = self.sessions.get_mut(&sid) else {
            self.buffer_pending(sid, n);
            return Applied::default();
        };
        let applied = entry.apply_event(n);
        self.publish(&applied.changes);
        applied
    }

    /// Stores an event for a session that is not loaded, evicting the oldest session's buffer
    /// when too many sessions are waiting.
    fn buffer_pending(&mut self, sid: String, n: Notification) {
        if !self.pending.contains_key(&sid) {
            if self.pending_order.len() >= MAX_PENDING_SESSIONS
                && let Some(old) = self.pending_order.pop_front()
            {
                self.pending.remove(&old);
            }
            self.pending_order.push_back(sid.clone());
        }
        let queue = self.pending.entry(sid).or_default();
        if queue.len() == EVENT_LOG_CAP {
            queue.pop_front();
        }
        queue.push_back(n);
    }

    /// Takes (and forgets) the buffered events of a session that just became loaded.
    pub(crate) fn take_pending(&mut self, sid: &str) -> Vec<Notification> {
        self.pending_order.retain(|s| s != sid);
        self.pending.remove(sid).map(Vec::from).unwrap_or_default()
    }

    /// Publishes `changes` and returns them wrapped as [`Applied`] with no actions.
    pub(crate) fn emit_all(&mut self, changes: Vec<Change>) -> Applied {
        self.publish(&changes);
        Applied {
            changes,
            actions: Vec::new(),
        }
    }

    /// Publishes `changes` to every live subscriber, in order. `Replaced` becomes a snapshot of
    /// the session as it is right now, which is why callers publish before applying anything that
    /// follows the replacement.
    pub(crate) fn publish(&mut self, changes: &[Change]) {
        self.subs.retain(|w| w.strong_count() > 0);
        if self.subs.is_empty() {
            return;
        }
        for change in changes {
            let delivery = match change {
                Change::Replaced { session_id } => match self.sessions.get(session_id) {
                    Some(entry) => Delivery::Snapshot(Box::new(entry.view())),
                    None => continue,
                },
                other => Delivery::Change(other.clone()),
            };
            for sub in self.subs.iter().filter_map(Weak::upgrade) {
                if sub.push(delivery.clone()) && !self.touched.iter().any(|t| Arc::ptr_eq(t, &sub))
                {
                    self.touched.push(sub);
                }
            }
        }
    }
}
