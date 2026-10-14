//! Event forwarding: one task per UI subscription that pulls [`CoreEvent`]s from the core and hands
//! them to a sink (a Tauri `Channel`, in the app).
//!
//! Depends on: vw-core-api (`CoreSubscription`, `CoreEvent`), tokio. Called by: [`crate::Host`].
//! Invariants:
//! - At most one forwarder runs: a new subscription (the UI reloaded and asks again) aborts the
//!   previous task first, so the UI never receives duplicated streams.
//! - The task ends when the sink reports it is gone (`false`), when the core is gone, or when it
//!   is replaced; it never outlives [`Forwarder::stop`].
//! - Order is the subscription's order; the sink is called from one task, so it sees events in
//!   sequence.

use std::sync::{Mutex, PoisonError};

use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use vw_core_api::{CoreEvent, CoreSubscription};

/// Owns the single running forwarding task.
#[derive(Default)]
pub struct Forwarder {
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Forwarder {
    /// Starts forwarding `subscription` to `sink` on `runtime`, replacing any earlier forwarder.
    /// `sink` returns `false` when its receiver is gone, which ends the task.
    pub fn replace<S>(&self, runtime: &Handle, mut subscription: CoreSubscription, mut sink: S)
    where
        S: FnMut(CoreEvent) -> bool + Send + 'static,
    {
        let task = runtime.spawn(async move {
            while let Some(event) = subscription.next().await {
                if !sink(event) {
                    break;
                }
            }
        });
        let previous = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(task);
        if let Some(previous) = previous {
            previous.abort();
        }
    }

    /// Stops the running forwarder, if any.
    pub fn stop(&self) {
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        self.stop();
    }
}
