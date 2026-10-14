//! Linux confirmation window with cancellation tied to the awaiting request.
//!
//! Depends on: GTK on Tauri's main thread and a Tokio one-shot reply. Called by: `dialogs::show_dialog`.
//! A dropped request destroys its own window; late answers cannot approve a canceled action.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gtk::prelude::*;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::oneshot;

thread_local! {
    static WINDOWS: RefCell<HashMap<u64, gtk::MessageDialog>> = RefCell::new(HashMap::new());
}
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
            if let Some(dialog) = WINDOWS.with(|windows| windows.borrow_mut().remove(&id)) {
                // A GTK dialog may handle close as a response without destroying itself.
                // Hide first so a canceled request cannot leave an actionable window behind.
                dialog.hide();
                dialog.close();
            }
        });
    }
}

/// Shows one confirmation and returns a pressed button. Dropping this future closes its window.
pub(super) async fn show_dialog<R: Runtime>(
    app: &AppHandle<R>,
    text: String,
    labels: &[String],
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
            let parent = app_for_window
                .get_webview_window("main")
                .and_then(|window| window.gtk_window().ok());
            let dialog = gtk::MessageDialog::new(
                parent.as_ref(),
                gtk::DialogFlags::MODAL,
                gtk::MessageType::Warning,
                gtk::ButtonsType::None,
                "Vibe Work asks for your confirmation",
            );
            dialog.set_secondary_text(Some(&text));
            dialog.set_title("Vibe Work asks for your confirmation");
            for (index, label) in labels.iter().enumerate() {
                let Ok(response) = u16::try_from(index + 1) else {
                    return;
                };
                dialog.add_button(label, gtk::ResponseType::Other(response));
            }
            dialog.set_default_response(gtk::ResponseType::Other(1));
            let send = RefCell::new(Some(send));
            let canceled_on_response = canceled.clone();
            dialog.connect_response(move |_, response| {
                let answer = if canceled_on_response.load(Ordering::Acquire) {
                    None
                } else {
                    match response {
                        gtk::ResponseType::Other(index) if index > 0 => {
                            labels.get(usize::from(index - 1)).cloned()
                        }
                        _ => None,
                    }
                };
                let send = send.borrow_mut().take();
                if let Some(dialog) = WINDOWS.with(|windows| windows.borrow_mut().remove(&id)) {
                    dialog.hide();
                    dialog.close();
                }
                if let Some(send) = send {
                    let _ = send.send(answer);
                }
            });
            WINDOWS.with(|windows| windows.borrow_mut().insert(id, dialog.clone()));
            dialog.show_all();
        })
        .is_err()
    {
        return None;
    }
    let answer = receive.await.ok().flatten();
    drop(owner);
    answer
}
