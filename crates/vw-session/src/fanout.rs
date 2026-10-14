//! Subscriber queues: snapshot first, then ordered changes; a slow subscriber is resnapshotted.
//!
//! Responsibility: give each subscriber a bounded FIFO of [`Delivery`] items that the store fills
//! while it holds its lock. `push` never blocks and never allocates beyond the capacity: when a
//! subscriber falls behind by more than `capacity` items its queue is dropped and replaced, on its
//! next read, by a fresh [`Delivery::Snapshot`] of every session (changes in between are covered
//! by those snapshots). Memory per subscriber is therefore bounded.
//! Depends on: `change`, `store` (the handle, for the lagged rebuild).
//! Called by: `store` (producer side), the UI bridge / Debug API (consumer side).
//! Lock order: store lock, then queue lock. Consumers never hold the queue lock while taking the
//! store lock.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use crate::change::{Change, SessionView};
use crate::store::SessionStore;

/// One item a subscriber receives.
#[derive(Debug, Clone, PartialEq)]
pub enum Delivery {
    /// Full state of one session; replaces whatever the subscriber holds for it.
    Snapshot(Box<SessionView>),
    /// An incremental change, to be applied after the snapshot it follows.
    Change(Change),
}

/// Called (without store locks held) after items were queued for a subscriber.
pub type Waker = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Queue {
    items: VecDeque<Delivery>,
    /// Set when the queue overflowed: pending items were dropped and further changes are
    /// skipped until the consumer reads and receives fresh snapshots.
    lagged: bool,
    /// How many times this subscriber overflowed.
    lag_events: u64,
}

/// State shared between the store (producer) and one [`Subscription`] (consumer).
pub(crate) struct SubShared {
    queue: Mutex<Queue>,
    waker: Mutex<Option<Waker>>,
    capacity: usize,
}

impl SubShared {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(Queue::default()),
            waker: Mutex::new(None),
            capacity: capacity.max(1),
        }
    }

    /// Queues an item. Over capacity, the queue is dropped and flagged lagged instead of
    /// growing. Returns true when something was queued (so the waker should run).
    pub(crate) fn push(&self, item: Delivery) -> bool {
        let mut q = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        if q.lagged {
            return false;
        }
        if q.items.len() >= self.capacity {
            q.items.clear();
            q.lagged = true;
            q.lag_events += 1;
            return true;
        }
        q.items.push_back(item);
        true
    }

    /// Queues the initial snapshots, ignoring the capacity (bounded by the session count).
    pub(crate) fn push_initial(&self, snapshots: Vec<SessionView>) {
        let mut q = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        q.items.extend(
            snapshots
                .into_iter()
                .map(|v| Delivery::Snapshot(Box::new(v))),
        );
    }

    /// Replaces the queue by snapshots of every session after a lag. Called under the store lock.
    pub(crate) fn rebuild(&self, snapshots: Vec<SessionView>) {
        let mut q = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        if q.lagged {
            q.items = snapshots
                .into_iter()
                .map(|v| Delivery::Snapshot(Box::new(v)))
                .collect();
            q.lagged = false;
        }
    }

    pub(crate) fn wake(&self) {
        if let Some(w) = &*self.waker.lock().unwrap_or_else(PoisonError::into_inner) {
            w();
        }
    }
}

/// A consumer handle. Dropping it unsubscribes (the store prunes dead queues on its next write).
pub struct Subscription {
    pub(crate) shared: Arc<SubShared>,
    pub(crate) store: SessionStore,
}

impl Subscription {
    /// Installs a callback invoked after items are queued, e.g. to wake an async task. It runs on
    /// the thread that called `apply`/`load`, after all store locks were released; keep it cheap.
    pub fn set_waker(&self, waker: impl Fn() + Send + Sync + 'static) {
        *self
            .shared
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(waker));
    }

    /// Next item, or `None` when the queue is empty. After an overflow the first call returns a
    /// fresh snapshot of every session, then ordinary changes resume.
    #[must_use]
    pub fn try_next(&self) -> Option<Delivery> {
        let lagged = {
            let mut q = self
                .shared
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if !q.lagged {
                return q.items.pop_front();
            }
            q.lagged
        };
        if lagged {
            self.store.rebuild_subscriber(&self.shared);
        }
        self.shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .items
            .pop_front()
    }

    /// Drains everything currently queued.
    #[must_use]
    pub fn drain(&self) -> Vec<Delivery> {
        std::iter::from_fn(|| self.try_next()).collect()
    }

    /// How many times this subscriber fell behind and was resnapshotted.
    #[must_use]
    pub fn lag_events(&self) -> u64 {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lag_events
    }
}
