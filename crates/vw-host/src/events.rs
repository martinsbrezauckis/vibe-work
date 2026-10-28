//! Events that come from the host itself rather than from Vibe: OS notification clicks and physical browser
//! context-menu save requests.
//!
//! Depends on: tokio (`broadcast`), serde. Called by: [`crate::Host`] (publishes, forwards), `app`
//! (`host_subscribe` command carries them over a Tauri channel).
//! Invariants:
//! - Wire forms `{"type": "openSession", "sessionId": "...", "notificationId"?: number}`,
//!   `{"type": "openUpdates"}`, and
//!   `{"type": "browserSaveContext", "sessionId": "..."}`; the UI mirrors them in
//!   `ui/src/protocol/client/types.ts` and a private test pins the JSON.
//! - Delivery is best effort and ordered; a subscriber that is gone ends its forwarding task.
//! - One forwarder at a time (a reload of the UI replaces the previous one).

use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Something the UI should react to that did not come from Vibe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostEvent {
    /// The user clicked the OS notification of this session: open it.
    OpenSession {
        /// Session to open.
        session_id: String,
        /// In-app history entry to select; absent for older callers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notification_id: Option<u32>,
    },
    /// The user activated an OS update notice: show Updates settings.
    OpenUpdates,
    /// A physical Save to vault click in this session's child browser webview.
    BrowserSaveContext {
        /// Session whose browser held the one-use choice; no page value crosses this event.
        session_id: String,
    },
    /// A physical Point Vibe at this click in the page menu of a Browser page: the session's app preview or one of
    /// its web pages. The window runs the pick of that page, which reads the selection in the page.
    BrowserPointContext {
        /// The page the click was in: the session id for its app preview, the page key for a web page; no page
        /// value crosses this event.
        session_id: String,
    },
}

/// The host's event channel.
pub(crate) struct HostEvents {
    sender: broadcast::Sender<HostEvent>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl HostEvents {
    pub(crate) fn new() -> Self {
        Self {
            sender: broadcast::channel(64).0,
            task: Mutex::new(None),
        }
    }

    /// A handle that can publish from any thread (the notification click runs on one).
    pub(crate) fn publisher(&self) -> broadcast::Sender<HostEvent> {
        self.sender.clone()
    }

    /// Forwards events to `sink` until it returns false; replaces an earlier forwarder.
    pub(crate) fn forward<S>(&self, runtime: &Handle, mut sink: S)
    where
        S: FnMut(HostEvent) -> bool + Send + 'static,
    {
        let mut receiver = self.sender.subscribe();
        let task = runtime.spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        if !sink(event) {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        if let Some(previous) = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(task)
        {
            previous.abort();
        }
    }

    pub(crate) fn stop(&self) {
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
