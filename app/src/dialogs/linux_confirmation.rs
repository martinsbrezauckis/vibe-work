//! Linux confirmation wiring with cancellation tied to the awaiting request.
//!
//! Depends on: Tauri main-thread scheduling, vw-platform windows, Tokio replies and vw-host copy. Called by:
//! `dialogs::show_dialog`.
//! A dropped request destroys its own window; late answers cannot approve a canceled action. The last button (the
//! declining one) is the default and has the focus; the question is the window's primary text.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tauri::{AppHandle, Runtime};
use tokio::sync::oneshot;
use vw_platform::confirm_window::{ConfirmSpec, linux as window};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct WindowOwner<R: Runtime> {
    app: AppHandle<R>,
    id: u64,
    canceled: Arc<AtomicBool>,
}

impl<R: Runtime> Drop for WindowOwner<R> {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Release);
        let id = self.id;
        let _ = self.app.run_on_main_thread(move || {
            window::close(id);
        });
    }
}

/// Shows one confirmation and returns a pressed button. Dropping this future closes its window.
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
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let canceled = Arc::new(AtomicBool::new(false));
    let owner = WindowOwner {
        app: app.clone(),
        id,
        canceled: canceled.clone(),
    };
    let (send, receive) = oneshot::channel();
    let labels = labels.to_vec();
    let app_for_window = app.clone();
    if app
        .run_on_main_thread(move || {
            // Cancellation can race with posting this closure. A canceled request must never show a dialog.
            if canceled.load(Ordering::Acquire) {
                return;
            }
            let parent =
                crate::main_window(&app_for_window).and_then(|window| window.gtk_window().ok());
            // The question is the bold first line (Orca reads it right after the window title), the rest below it.
            let (heading, rest) = vw_host::dialog_heading(&text);
            let spec = ConfirmSpec {
                title: vw_host::DIALOG_TITLE.to_owned(),
                heading,
                body: rest,
                buttons: labels,
            };
            window::show(
                id,
                parent.as_ref(),
                &spec,
                modal,
                &canceled,
                Box::new(move |answer| {
                    let _ = send.send(answer);
                }),
            );
        })
        .is_err()
    {
        return None;
    }
    let handle = app.clone();
    let _listed = super::listed(
        ask,
        Arc::new(move || {
            handle
                .run_on_main_thread(move || {
                    window::raise(id);
                })
                .is_ok()
        }),
    );
    let answer = receive.await.ok().flatten();
    drop(owner);
    answer
}

#[cfg(test)]
#[path = "linux_confirmation_unit_tests.rs"]
mod tests;
