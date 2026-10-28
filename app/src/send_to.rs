//! macOS "Send to Vibe Work": the services provider that turns Finder's and other apps' service requests into
//! hand-overs (`vw_host::hand_over`) in the app's inbox; the window takes them (`commands::hand_over_take`).
//!
//! Depends on: vw-host (`hand_over`), vw-platform (`send_to::mac`). Called by: `lib.rs` setup.
//! Invariant: a request only becomes chips in the message box; nothing is sent.

use std::sync::Arc;

use vw_host::hand_over::HandOverInbox;

/// macOS: makes the app the provider of its two services; each request becomes a hand-over and the window comes
/// to the front. Call on the main thread in setup.
pub fn register_services(app: &tauri::AppHandle, inbox: Arc<HandOverInbox>) {
    use vw_host::hand_over::{HandOver, HandOverText};
    use vw_platform::send_to::mac::{ServiceRequest, register_provider};
    let app = app.clone();
    let registered = register_provider(move |request| {
        let hand_over = match request {
            ServiceRequest::Files(paths) => HandOver {
                paths,
                ..HandOver::default()
            },
            ServiceRequest::Text { text, from } => HandOver {
                text: Some(HandOverText { text, from }),
                ..HandOver::default()
            },
        };
        inbox.push(hand_over.admit());
        if let Some(window) = crate::main_window(&app) {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
    if !registered {
        tracing::warn!("the Send to Vibe Work services were not registered");
    }
}
