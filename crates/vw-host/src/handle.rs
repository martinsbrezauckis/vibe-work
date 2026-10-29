//! `HostHandle`: lets the window open at once while the host starts in the background.
//!
//! Starting the core runs the Vibe handshake, which can take seconds (Python start) or, when
//! something is wrong, up to its 30 s timeout. The app must not hold the window back for that, so
//! it creates a handle, starts the host in a task, and every Tauri command waits on the handle.
//!
//! Depends on: tokio (`watch`), [`crate::Host`]. Called by: `app`. Invariants: the host is set
//! exactly once; waiting after it is set returns immediately; a command that arrives before the
//! host is ready waits instead of failing. Each main-document load gets the next generation before any of its
//! window commands can start; a window command keeps the generation it started under.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::runtime::Handle;
use tokio::sync::watch;
use vw_core_api::{Command, CommandResult, CoreError};

use crate::host::{Host, HostConfig};

/// A cell that is filled once, with the started [`Host`].
#[derive(Clone)]
pub struct HostHandle {
    slot: Arc<watch::Sender<Option<Arc<Host>>>>,
    ui_load: Arc<Mutex<watch::Receiver<bool>>>,
    ui_generation: Arc<AtomicU64>,
}

impl Default for HostHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl HostHandle {
    /// An empty handle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slot: Arc::new(watch::Sender::new(None)),
            ui_load: Arc::new(Mutex::new(watch::channel(true).1)),
            ui_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Starts the host on `runtime` and fills the handle when it is ready.
    pub fn start_in_background(&self, runtime: &Handle, config: HostConfig) {
        let slot = self.slot.clone();
        runtime.spawn(async move {
            let host = Arc::new(Host::start(config).await);
            slot.send_replace(Some(host));
        });
    }

    /// Queues a native-placement reset for every main-document load, independently of renderer cleanup.
    /// The barrier and the new generation are installed synchronously, so new window commands cannot overtake the
    /// reset, and placements of commands the replaced document started are refused once it ran.
    pub fn main_ui_load_started(&self, runtime: &Handle) {
        let (done, pending) = watch::channel(false);
        let (previous, generation) = {
            let mut slot = self.ui_load.lock().unwrap_or_else(PoisonError::into_inner);
            let generation = self.ui_generation.fetch_add(1, Ordering::SeqCst) + 1;
            (std::mem::replace(&mut *slot, pending), generation)
        };
        let handle = self.clone();
        runtime.spawn(async move {
            wait_for_load(previous).await;
            let host = handle.get().await;
            if let Some(core) = host.core() {
                core.reset_native_views(generation).await;
            }
            done.send_replace(true);
        });
    }

    /// The generation of the current main document: a window command takes it when it starts, before any wait.
    #[must_use]
    pub fn window_generation(&self) -> u64 {
        self.ui_generation.load(Ordering::SeqCst)
    }

    /// Runs one main-window command ([`Host::execute_window`]) after the placement resets of earlier main-document
    /// loads. It keeps the generation it started under, so once a later load has reset the native placements, its
    /// Browser, Project and File viewer placements place nothing.
    ///
    /// # Errors
    /// As [`Host::execute_window`].
    pub async fn execute_window(&self, command: Command) -> Result<CommandResult, CoreError> {
        let generation = self.window_generation();
        vw_core_api::for_window_generation(generation, async {
            self.get_for_window().await.execute_window(command).await
        })
        .await
    }

    /// The started host after native placements from preceding main-document loads have been cleared.
    async fn get_for_window(&self) -> Arc<Host> {
        let host = self.get().await;
        self.wait_for_ui_load().await;
        host
    }

    async fn wait_for_ui_load(&self) {
        loop {
            let pending = self
                .ui_load
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            wait_for_load(pending.clone()).await;
            if pending.same_channel(&self.ui_load.lock().unwrap_or_else(PoisonError::into_inner)) {
                return;
            }
        }
    }

    /// The host, waiting until it has started.
    pub async fn get(&self) -> Arc<Host> {
        let mut receiver = self.slot.subscribe();
        loop {
            if let Some(host) = receiver.borrow_and_update().clone() {
                return host;
            }
            // The sender is owned by this handle (`self.slot`), so `changed` cannot fail while
            // we are borrowed; if it ever did, looping again would simply re-check the cell.
            let _ = receiver.changed().await;
        }
    }

    /// The host if it has started already.
    #[must_use]
    pub fn try_get(&self) -> Option<Arc<Host>> {
        self.slot.borrow().clone()
    }
}

async fn wait_for_load(mut pending: watch::Receiver<bool>) {
    while !*pending.borrow_and_update() {
        if pending.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::task::{Context, Waker};

    #[tokio::test]
    async fn each_main_document_load_starts_the_next_generation_at_once() {
        let handle = HostHandle::new();
        assert_eq!(handle.window_generation(), 0);
        let runtime = Handle::current();
        handle.main_ui_load_started(&runtime);
        assert_eq!(handle.window_generation(), 1);
        handle.main_ui_load_started(&runtime);
        assert_eq!(
            handle.window_generation(),
            2,
            "a command of the new document never takes the old generation, even before any reset ran"
        );
    }

    #[tokio::test]
    async fn window_commands_wait_for_the_latest_main_document_reset() {
        let handle = HostHandle::new();
        let (first_done, first) = watch::channel(false);
        *handle.ui_load.lock().unwrap() = first;
        let mut waiting = std::pin::pin!(handle.wait_for_ui_load());
        let mut context = Context::from_waker(Waker::noop());
        assert!(waiting.as_mut().poll(&mut context).is_pending());

        let (last_done, last) = watch::channel(false);
        *handle.ui_load.lock().unwrap() = last;
        first_done.send_replace(true);
        assert!(waiting.as_mut().poll(&mut context).is_pending());
        last_done.send_replace(true);
        assert!(waiting.as_mut().poll(&mut context).is_ready());
    }
}
