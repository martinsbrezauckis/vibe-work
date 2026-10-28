//! `CoreSubscription`: one client's stream of core events, merging the store's session stream
//! (snapshots, then ordered changes) with the core's own events, optionally for one session.
//!
//! Depends on: vw-session (`Subscription`), tokio (`broadcast`, `Notify`), `core::Shared`,
//! `event`. Called by: vw-debug-api (`GET /events`), the Tauri bridge, tests.
//! Invariants:
//! - Store items keep their order; the first items are snapshots of every loaded session (or of
//!   the one followed session).
//! - Lag never loses state silently: when the store resnapshots this subscriber, or the core-event
//!   buffer overflows, the next item is `lagged`, followed by fresh snapshots of every followed
//!   session (the store's rule, also applied after a core-event overflow).
//! - Memory is bounded: the store queue by `subscriber_capacity`, core events by
//!   `event_capacity`.

use std::collections::BTreeSet;
use std::sync::{Arc, Weak};

use tokio::sync::{Notify, broadcast};
use vw_session::{SessionStore, Subscription};

use crate::core::Shared;
use crate::event::{AlertKind, CoreEvent};
use crate::view::SessionSnapshot;

/// A live subscription. Dropping it unsubscribes.
pub struct CoreSubscription {
    shared: Weak<Shared>,
    store: Subscription,
    store_handle: SessionStore,
    events: broadcast::Receiver<CoreEvent>,
    wake: Arc<Notify>,
    session: Option<String>,
    lag_seen: u64,
    /// Snapshots still to hand out after a core-event overflow.
    refill: Vec<CoreEvent>,
    cleanup_notices: Vec<CoreEvent>,
    seen_cleanup: BTreeSet<(String, bool)>,
}

impl CoreSubscription {
    pub(crate) fn new(shared: &Arc<Shared>, session: Option<String>) -> Self {
        // Core events first: anything emitted after this point is received, and the store
        // snapshot taken next is at least as new as any event about it.
        let events = shared.events.subscribe();
        let store = shared.store.subscribe(shared.config.subscriber_capacity);
        let wake = Arc::new(Notify::new());
        let waker = wake.clone();
        store.set_waker(move || waker.notify_one());
        let mut cleanup_notices = shared.temporary.notices();
        cleanup_notices.extend(shared.asks.replay());
        if shared
            .network_notice
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            cleanup_notices.push(crate::network::rejection_notice(
                shared.config.keys.as_deref(),
            ));
        }
        cleanup_notices.extend(shared.guard_refusals.events().into_iter().rev());
        cleanup_notices.push(CoreEvent::SecretScan {
            view: shared.secret_scan.view(),
        });
        if let Some(keys) = &shared.config.keys {
            cleanup_notices.extend(keys.remote_cleanup_notices().into_iter().map(|message| {
                CoreEvent::Alert {
                    session_id: None,
                    kind: AlertKind::RemoteFileCleanupFailed,
                    message,
                }
            }));
        }
        Self {
            shared: Arc::downgrade(shared),
            store,
            store_handle: shared.store.clone(),
            events,
            wake,
            session,
            lag_seen: 0,
            refill: Vec::new(),
            // Replayed to every new subscriber: cleanup warnings (local and Mistral files) and session holders.
            cleanup_notices: cleanup_notices
                .into_iter()
                .chain(shared.holders.events())
                .collect(),
            seen_cleanup: BTreeSet::new(),
        }
    }

    fn wanted(&self, event: &CoreEvent) -> bool {
        self.session.as_deref().is_none_or(|id| event.concerns(id))
    }

    fn accept(&mut self, event: &CoreEvent) -> bool {
        if !self.wanted(event) {
            return false;
        }
        if let CoreEvent::Alert {
            session_id: Some(id),
            kind,
            ..
        } = event
            && matches!(
                kind,
                AlertKind::TemporaryCleanupFailed | AlertKind::TemporaryCleanupRecovered
            )
        {
            return self
                .seen_cleanup
                .insert((id.clone(), *kind == AlertKind::TemporaryCleanupRecovered));
        }
        true
    }

    /// Display modes come from the core's record.
    fn with_access(&self, mut event: CoreEvent) -> CoreEvent {
        let Some(shared) = self.shared.upgrade() else {
            return event;
        };
        match &mut event {
            CoreEvent::Snapshot { snapshot } => {
                crate::agent_tools::shell_commands::project(&shared, &mut snapshot.state);
                shared
                    .protected_changes
                    .overlay(&snapshot.session_id, &mut snapshot.state);
                shared
                    .chrome_control
                    .overlay(&snapshot.session_id, &mut snapshot.state);
                crate::ask::tool_calls::overlay(&shared, &snapshot.session_id, &mut snapshot.state);
                snapshot.access_mode = crate::restore::display_mode(
                    &shared,
                    &snapshot.session_id,
                    &snapshot.state["session"],
                );
            }
            CoreEvent::Change {
                change:
                    crate::event::SessionChange::RuntimeUpdated {
                        session_id,
                        runtime,
                        access_mode,
                    },
            } => {
                *access_mode = crate::restore::display_mode(
                    &shared,
                    session_id,
                    &serde_json::json!({"agent": runtime["activeAgent"]}),
                );
            }
            _ => {}
        }
        event
    }

    /// Fresh snapshots of every followed session, in id order.
    fn snapshots(&self) -> Vec<CoreEvent> {
        let mut snapshots: Vec<_> = self
            .store_handle
            .views()
            .into_iter()
            .map(|v| {
                self.with_access(CoreEvent::Snapshot {
                    snapshot: SessionSnapshot::from(v),
                })
            })
            .filter(|e| self.wanted(e))
            .collect();
        if let Some(shared) = self.shared.upgrade() {
            snapshots.extend(
                shared
                    .guard_refusals
                    .events()
                    .into_iter()
                    .filter(|event| self.wanted(event)),
            );
            snapshots.extend(
                shared
                    .holders
                    .events()
                    .into_iter()
                    .filter(|event| self.wanted(event)),
            );
        }
        snapshots
    }

    /// The next item without waiting, if one is ready.
    pub fn try_next(&mut self) -> Option<CoreEvent> {
        if let Some(event) = self.refill.pop() {
            return Some(event);
        }
        let lag = self.store.lag_events();
        if lag > self.lag_seen {
            self.lag_seen = lag;
            return Some(CoreEvent::Lagged {});
        }
        while let Some(delivery) = self.store.try_next() {
            if let Some(event) = CoreEvent::from_delivery(delivery)
                && self.wanted(&event)
            {
                return Some(self.with_access(event));
            }
        }
        while let Some(event) = self.cleanup_notices.pop() {
            if self.accept(&event) {
                return Some(event);
            }
        }
        loop {
            match self.events.try_recv() {
                Ok(event) if self.accept(&event) => return Some(event),
                Ok(_) => {}
                Err(broadcast::error::TryRecvError::Lagged(_)) => {
                    let mut refill = self.snapshots();
                    refill.reverse();
                    self.refill = refill;
                    return Some(CoreEvent::Lagged {});
                }
                Err(_) => return None,
            }
        }
    }

    /// The next item, waiting until one is ready. `None` when the core is gone.
    pub async fn next(&mut self) -> Option<CoreEvent> {
        loop {
            if let Some(event) = self.try_next() {
                return Some(event);
            }
            tokio::select! {
                () = self.wake.notified() => {}
                received = self.events.recv() => match received {
                    Ok(event) if self.accept(&event) => return Some(event),
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let mut refill = self.snapshots();
                        refill.reverse();
                        self.refill = refill;
                        return Some(CoreEvent::Lagged {});
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
            }
        }
    }
}
