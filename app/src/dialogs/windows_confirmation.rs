//! Windows confirmation window with cancellation tied to the awaiting request: the app side of
//! `vw_platform::confirm_window` (a task dialog modal over the main window).
//!
//! Depends on: Tauri's main window handle, vw-platform (the window), vw-host (heading, button order), a Tokio one-shot
//! reply. Called by: `dialogs::show_dialog`.
//! The declining button is first, the default and focused (Return); Escape and the close button decline; a dropped
//! request (a cancelled call) closes the window, and a late press never approves.

use tauri::{AppHandle, Runtime};
use tokio::sync::oneshot;
use vw_platform::confirm_window::{self, ConfirmSpec};

/// Shows one confirmation and returns the pressed button's label; `None` when declined without a button (Escape,
/// the close button), closed, failed, or when there are not 1 to 3 labels. Dropping this future closes its window.
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
    // Modal over the main window when it exists; an agent's request is a window of its own (no owner to disable).
    let parent = crate::main_window(app)
        .filter(|_| modal)
        .and_then(|window| window.hwnd().ok())
        .map_or(0, |hwnd| hwnd.0 as isize);
    let (send, receive) = oneshot::channel();
    let reply = Box::new(move |index: Option<usize>| {
        if send.send(index).is_err() {
            tracing::info!("native answer ignored after the permission wait closed");
        }
    });
    // Held until the answer: dropping this future drops it, which closes the window.
    let open = match confirm_window::show(parent, &spec, reply) {
        Ok(open) => open,
        Err(error) => {
            tracing::warn!(%error, "confirmation window not shown; counted as declined");
            return None;
        }
    };
    let raiser = open.raiser();
    let _listed = super::listed(ask, std::sync::Arc::new(move || raiser.raise()));
    let index = receive.await.ok().flatten();
    index.and_then(|i| ordered.get(i).cloned())
}
