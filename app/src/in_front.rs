//! Whether one of the app's own windows is in front, for `notify_session`'s `skipWhenAppInFront`: the
//! webview reports no focus while the app's own confirmation window has it, so the host decides from the OS.
//! Windows and macOS ask the OS (`vw_platform::own_app_in_front`); Linux asks GTK on the UI thread whether any of the
//! app's top-level windows (the main window or the confirmation window) is active. When no answer comes in time the
//! answer is "not in front", so the notification is shown (never a missed request).
//!
//! Depends on: vw-platform, tauri (`run_on_main_thread`), gtk (Linux). Called by: `commands::notify_session`.

use std::time::Duration;

/// How long to wait for the UI thread's answer (Linux).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const ANSWER_WITHIN: Duration = Duration::from_millis(500);

/// True when one of the app's own windows is in front.
pub fn app_in_front(app: &tauri::AppHandle) -> bool {
    if let Some(answer) = vw_platform::own_app_in_front() {
        return answer;
    }
    toolkit_in_front(app)
}

#[cfg(target_os = "linux")]
fn toolkit_in_front(app: &tauri::AppHandle) -> bool {
    use gtk::prelude::{Cast, GtkWindowExt};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let asked = app.run_on_main_thread(move || {
        let active = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .any(|w| w.is_active());
        let _ = tx.send(active);
    });
    asked.is_ok() && rx.recv_timeout(ANSWER_WITHIN).unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn toolkit_in_front(_app: &tauri::AppHandle) -> bool {
    false
}
