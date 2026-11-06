//! Pump-owned resync requests: single flight, coalesced deadlines and budget-expiry recovery.
//! Depends on the parent pump, store and gate; removal/generation changes discard deferred work.

use super::{Pump, Resynced, Shared};
use crate::event::{AlertKind, CoreEvent};
use crate::resync_budget::Budget;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, atomic::Ordering};
use tracing::warn;
use vw_session::{Applied, LoadKind};

impl Pump {
    pub(super) fn resync_all(&mut self, shared: &Arc<Shared>) {
        for view in shared.store.views() {
            self.resync(shared, &view.session_id);
        }
    }

    /// Starts a `session/read` for `session_id`, or queues one more if a read is in flight.
    pub(super) fn resync(&mut self, shared: &Arc<Shared>, session_id: &str) {
        if self.in_flight.contains(session_id) {
            self.queued.insert(session_id.to_owned());
            return;
        }
        match self
            .budget
            .spend(session_id, tokio::time::Instant::now().into_std())
        {
            Budget::Allowed => {}
            Budget::Paused { first } => {
                if let Some(until) = self.budget.paused_until(session_id) {
                    self.later.insert(session_id.to_owned(), until.into());
                }
                if first {
                    warn!(
                        session_id,
                        "too many resyncs; pausing re-reads of this session"
                    );
                    shared.emit(CoreEvent::Alert {
                        session_id: Some(session_id.to_owned()),
                        kind: AlertKind::ResyncPaused,
                        message: "A session's data kept changing under the app, so it stopped re-reading it for a while. What you see may be out of date.".to_owned(),
                    });
                }
                return;
            }
        }
        // An immediate request also satisfies any previously deferred request.
        self.later.remove(session_id);
        self.in_flight.insert(session_id.to_owned());
        let shared = shared.clone();
        let session_id = session_id.to_owned();
        self.resyncs.spawn(async move {
            let applied = refresh(&shared, &session_id).await;
            Resynced {
                session_id,
                applied,
            }
        });
    }

    /// A resync finished: apply its follow-up actions and run a queued one. Returns false when
    /// the core is gone.
    pub(super) fn resynced(&mut self, done: &Resynced) -> bool {
        let Some(shared) = self.weak.upgrade() else {
            return false;
        };
        self.in_flight.remove(&done.session_id);
        if let Some(applied) = &done.applied {
            self.after_apply(&shared, applied);
        }
        if self.queued.remove(&done.session_id) {
            self.resync(&shared, &done.session_id);
        }
        true
    }
}

/// Waits for the earliest owned request. Dropping the pump or removing the entry cancels the wakeup.
pub(super) async fn next_deferred(later: &BTreeMap<String, tokio::time::Instant>) -> String {
    let Some((id, until)) = later.iter().min_by_key(|(_, until)| *until) else {
        return std::future::pending().await;
    };
    tokio::time::sleep_until(*until).await;
    id.clone()
}

/// `session/read` through the gate, merged into the store (routed to the session's channel when it
/// has one). `None` when the read failed (logged; the session stays marked for resync and the next
/// event retries). Counted in `Shared::resync_reads` (the status report shows it: asserts no
/// read happens while sessions merely run).
pub(crate) async fn refresh(shared: &Shared, session_id: &str) -> Option<Applied> {
    shared.resync_reads.fetch_add(1, Ordering::Relaxed);
    let params = json!({"sessionId": session_id});
    let reply = shared.gate.call("session/read", params, None).await;
    let state = match reply {
        Ok(reply) => reply.result.get("state").cloned(),
        Err(error) => {
            warn!(session_id, %error, "resync read failed");
            return None;
        }
    };
    let loaded = state.map(|mut s| {
        crate::title_guard::withhold_in(shared, &mut s);
        shared.store.load(s, LoadKind::Read)
    });
    match loaded {
        Some(Ok(applied)) => Some(applied),
        Some(Err(error)) => {
            warn!(session_id, %error, "resync result could not be loaded");
            None
        }
        None => {
            warn!(session_id, "resync result has no state");
            None
        }
    }
}
