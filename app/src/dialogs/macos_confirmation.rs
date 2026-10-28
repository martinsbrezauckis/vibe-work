//! macOS confirmation window with cancellation tied to the awaiting request: the app side of
//! `vw_platform::confirm_window` (an `AppKit` alert sheet on the main window, one at a time).
//!
//! Depends on: Tauri's main thread and main window, vw-platform (the window), vw-host (heading, button order), a Tokio
//! one-shot reply. Called by: `dialogs::show_dialog`.
//! The declining button is first (rightmost) with Return; Escape declines; a dropped request (a
//! cancelled call) closes its sheet or takes it out of the queue, and a late press never approves.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tauri::{AppHandle, Runtime};
use tokio::sync::oneshot;
use vw_platform::confirm_window::{self, ConfirmSpec};

/// Closes the window when the request is dropped. Main-thread work is queued in order, so this close always runs
/// after the show it follows; a request dropped before its show ran never shows anything.
struct WindowOwner<R: Runtime> {
    app: AppHandle<R>,
    /// The window's id once shown (0 = not shown).
    id: Arc<AtomicU64>,
    canceled: Arc<AtomicBool>,
}

impl<R: Runtime> Drop for WindowOwner<R> {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Release);
        let id = self.id.clone();
        let _ = self.app.run_on_main_thread(move || {
            let shown = id.load(Ordering::Acquire);
            if shown != 0 {
                confirm_window::close(shown);
            }
        });
    }
}

/// Shows one confirmation and returns the pressed button's label; `None` when declined without a button (Escape),
/// closed, failed, or when there are not 1 to 3 labels. Dropping this future closes its window.
pub(super) async fn show_dialog<R: Runtime>(
    app: &AppHandle<R>,
    text: String,
    labels: &[String],
    modal: bool,
    ask: Option<&str>,
) -> Option<String> {
    if !(1..=3).contains(&labels.len()) {
        return None;
    }
    let ordered = vw_host::system_dialog_order(labels);
    let (heading, body) = vw_host::dialog_heading(&text);
    let spec = ConfirmSpec {
        title: vw_host::DIALOG_TITLE.to_owned(),
        heading,
        body,
        buttons: ordered.clone(),
    };
    let owner = WindowOwner {
        app: app.clone(),
        id: Arc::new(AtomicU64::new(0)),
        canceled: Arc::new(AtomicBool::new(false)),
    };
    // A sheet on a hidden or background window could wait unseen: present the main window first. Native input itself
    // never brings a window forward or uses this path to obtain focus.
    if let Some(window) = crate::main_window(app) {
        let _ = window.show();
        let _ = window.set_focus();
    }
    let (send, receive) = oneshot::channel();
    let (id, canceled, handle) = (owner.id.clone(), owner.canceled.clone(), app.clone());
    let posted = app.run_on_main_thread(move || {
        if canceled.load(Ordering::Acquire) {
            return;
        }
        // Read here, on the main thread, where the window cannot go away meanwhile.
        // A sheet on the main window; an agent's request is a window of its own, so the main window stays usable.
        let parent = crate::main_window(&handle)
            .filter(|_| modal)
            .and_then(|window| window.ns_window().ok())
            .unwrap_or(std::ptr::null_mut());
        let reply = Box::new(move |index: Option<usize>| {
            let _ = send.send(index);
        });
        match confirm_window::show(parent, &spec, reply) {
            Ok(shown) => id.store(shown, Ordering::Release),
            Err(error) => {
                tracing::warn!(%error, "confirmation window not shown; counted as declined");
            }
        }
    });
    if posted.is_err() {
        return None;
    }
    let (shown, handle) = (owner.id.clone(), app.clone());
    let _listed = super::listed(
        ask,
        Arc::new(move || {
            let shown = shown.clone();
            handle
                .run_on_main_thread(move || {
                    let _ = confirm_window::raise(shown.load(Ordering::Acquire));
                })
                .is_ok()
        }),
    );
    let index = receive.await.ok().flatten();
    drop(owner);
    index.and_then(|i| ordered.get(i).cloned())
}
