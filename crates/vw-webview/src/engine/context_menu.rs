//! Polls only interactive browser mounts for a physical page context-menu choice.
//!
//! Depends on the page-init script's one-bit intent accessor and a host callback. Called by the preview
//! engine on mount and close. The payload contains a session id only; the selected value remains inside the page.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tauri::{Webview, Wry};
use vw_preview::MountId;

pub(super) type Sink = std::sync::Arc<dyn Fn(String) + Send + Sync>;

const POLL: Duration = Duration::from_millis(150);
const ANSWER_WAIT: Duration = Duration::from_secs(4);
const INTENT: &str = "!!window.__vw_context_menu_intent?.take()";

/// One owned poll task per mounted browser, aborted on close or engine drop.
#[derive(Default)]
pub(super) struct Pollers {
    tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl Pollers {
    /// Starts an intent poll for one browser mount, replacing a stale task for its label.
    pub(super) fn start(&self, webview: Webview<Wry>, id: &MountId, sink: Sink) {
        let session_id = id.session_id.clone();
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(POLL);
            loop {
                tick.tick().await;
                let (tx, rx) = tokio::sync::oneshot::channel();
                let tx = Mutex::new(Some(tx));
                if webview
                    .eval_with_callback(INTENT, move |answer| {
                        if let Some(tx) = tx.lock().unwrap_or_else(PoisonError::into_inner).take() {
                            let _ = tx.send(answer);
                        }
                    })
                    .is_err()
                {
                    break;
                }
                if let Ok(Ok(answer)) = tokio::time::timeout(ANSWER_WAIT, rx).await
                    && answer == "true"
                {
                    sink(session_id.clone());
                }
            }
        });
        if let Some(previous) = self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.label.clone(), task)
        {
            previous.abort();
        }
    }

    /// Stops a browser mount's poll immediately.
    pub(super) fn stop(&self, label: &str) {
        if let Some(task) = self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(label)
        {
            task.abort();
        }
    }
}

impl Drop for Pollers {
    fn drop(&mut self) {
        for (_, task) in self
            .tasks
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
        {
            task.abort();
        }
    }
}
