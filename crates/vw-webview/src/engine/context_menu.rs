//! Polls only interactive browser mounts for a physical page context-menu choice (Save to vault, Point Vibe at
//! this).
//!
//! Depends on the page-init script's one-bit intent accessors and a host callback. Called by the preview
//! engine on mount and close. The payload contains a session id and which item was clicked only; the selected
//! value remains inside the page.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tauri::{Webview, Wry};
use vw_preview::MountId;

pub(super) type Sink = std::sync::Arc<dyn Fn(String, PageMenuChoice) + Send + Sync>;

/// Which item of the page's own right-click menu a person clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMenuChoice {
    /// Save to vault: the core still reads the one-use choice and asks before storing.
    SaveToVault,
    /// Point Vibe at this: the window runs the preview pick, which reads the selection in the page.
    PointVibe,
}

/// The choices one poll answer holds: a number, bit 1 Save to vault, bit 2 Point Vibe at this.
fn choices(answer: &str) -> Vec<PageMenuChoice> {
    let bits = answer.trim().parse::<u8>().unwrap_or(0);
    let mut out = Vec::new();
    if bits & 1 != 0 {
        out.push(PageMenuChoice::SaveToVault);
    }
    if bits & 2 != 0 {
        out.push(PageMenuChoice::PointVibe);
    }
    out
}

const POLL: Duration = Duration::from_millis(150);
const ANSWER_WAIT: Duration = Duration::from_secs(4);
const INTENT: &str = "(window.__vw_context_menu_intent?.take() ? 1 : 0) + (window.__vw_point_intent?.take() ? 2 : 0)";

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
                if let Ok(Ok(answer)) = tokio::time::timeout(ANSWER_WAIT, rx).await {
                    for choice in choices(&answer) {
                        sink(session_id.clone(), choice);
                    }
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

#[cfg(test)]
mod tests {
    use super::{PageMenuChoice, choices};

    const NONE: [PageMenuChoice; 0] = [];

    #[test]
    fn a_poll_answer_names_only_the_items_clicked() {
        assert_eq!(choices("0"), NONE);
        assert_eq!(choices("1"), [PageMenuChoice::SaveToVault]);
        assert_eq!(choices("2"), [PageMenuChoice::PointVibe]);
        assert_eq!(
            choices("3"),
            [PageMenuChoice::SaveToVault, PageMenuChoice::PointVibe]
        );
        // Anything else a page could make the answer look like counts as no click.
        assert_eq!(choices("true"), NONE);
        assert_eq!(choices("\"2\""), NONE);
        assert_eq!(choices(""), NONE);
    }
}
