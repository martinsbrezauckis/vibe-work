//! `HostHandle`: lets the window open at once while the host starts in the background.
//!
//! Starting the core runs the Vibe handshake, which can take seconds (Python start) or, when
//! something is wrong, up to its 30 s timeout. The app must not hold the window back for that, so
//! it creates a handle, starts the host in a task, and every Tauri command waits on the handle.
//!
//! Depends on: tokio (`watch`), [`crate::Host`]. Called by: `app`. Invariants: the host is set
//! exactly once; waiting after it is set returns immediately; a command that arrives before the
//! host is ready waits instead of failing.

use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::sync::watch;

use crate::host::{Host, HostConfig};

/// A cell that is filled once, with the started [`Host`].
#[derive(Clone)]
pub struct HostHandle {
    slot: Arc<watch::Sender<Option<Arc<Host>>>>,
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
