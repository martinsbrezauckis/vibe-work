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

use std::sync::Arc;

use tokio::sync::{Notify, broadcast};
use vw_session::{SessionStore, Subscription};

use crate::core::Shared;
use crate::event::CoreEvent;
use crate::view::SessionSnapshot;

/// A live subscription. Dropping it unsubscribes.
pub struct CoreSubscription {
    store: Subscription,
    store_handle: SessionStore,
    events: broadcast::Receiver<CoreEvent>,
    wake: Arc<Notify>,
    session: Option<String>,
    lag_seen: u64,
    /// Snapshots still to hand out after a core-event overflow.
    refill: Vec<CoreEvent>,
}

impl CoreSubscription {
    pub(crate) fn new(shared: &Shared, session: Option<String>) -> Self {
        // Core events first: anything emitted after this point is received, and the store
        // snapshot taken next is at least as new as any event about it.
        let events = shared.events.subscribe();
        let store = shared.store.subscribe(shared.config.subscriber_capacity);
        let wake = Arc::new(Notify::new());
        let waker = wake.clone();
        store.set_waker(move || waker.notify_one());
        Self {
            store,
            store_handle: shared.store.clone(),
            events,
            wake,
            session,
            lag_seen: 0,
            refill: Vec::new(),
        }
    }

    fn wanted(&self, event: &CoreEvent) -> bool {
        self.session.as_deref().is_none_or(|id| event.concerns(id))
    }

    /// Fresh snapshots of every followed session, in id order.
    fn snapshots(&self) -> Vec<CoreEvent> {
        self.store_handle
            .views()
            .into_iter()
            .map(|v| CoreEvent::Snapshot {
                snapshot: SessionSnapshot::from(v),
            })
            .filter(|e| self.wanted(e))
            .collect()
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
                return Some(event);
            }
        }
        loop {
            match self.events.try_recv() {
                Ok(event) if self.wanted(&event) => return Some(event),
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
                    Ok(event) if self.wanted(&event) => return Some(event),
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
