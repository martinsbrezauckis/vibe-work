//! Quit and Restart ask the person first while agent work runs. Every way the person ends the app goes through here:
//! the tray's Quit and Restart, the window's Quit command (Ctrl+Q, also pressed inside a built-in browser page), and an
//! exit the OS requests on the person's behalf
//! (macOS Cmd+Q and the app menu's Quit, closing the last window when the app does not stay in the tray).
//! The question is the app's own native confirmation (`dialogs::ask_person`): only the person can answer it, never the
//! agent or the Debug API. Cancel is the default and Escape cancels. With no running work nothing is asked.
//! Depends on: the host's core (`agent_work_running`), the native dialogs, the tray's restart, the browser views' key watch
//! (`vw_webview::take_quit_from_pages`, for Ctrl+Q in a page). Called by: tray, commands,
//! lib (window close and exit requests).

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager};

/// The question's words; used only by this dialog.
const QUIT_WHILE_WORKING: &str =
    "The agent in this chat is running. Are you sure you want to quit?";
const RESTART_WHILE_WORKING: &str =
    "The agent in this chat is running. Are you sure you want to restart?";
const QUIT_DETAIL: &str = "Quitting stops the agent's running work.";
const RESTART_DETAIL: &str = "Restarting stops the agent's running work.";
const QUIT: &str = "Quit";
const RESTART: &str = "Restart";
const CANCEL: &str = "Cancel";

/// How the app is to end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    Quit,
    Restart,
}

/// Takes the app's Quit shortcut (Ctrl+Q) from built-in browser pages: pressed while a page has the keyboard focus it
/// never reaches the page and ends the app as the window's own Ctrl+Q does (asking first while agent work runs).
pub(crate) fn take_from_pages(app: &AppHandle) {
    let app = app.clone();
    vw_webview::take_quit_from_pages(move || {
        let quit = app.clone();
        // Called on the key watch's thread: the request runs on the main thread.
        if let Err(error) = app.run_on_main_thread(move || request(&quit, End::Quit)) {
            tracing::warn!("Quit from a browser page: {error}");
        }
    });
}

/// One question is open at a time: a second Quit while it shows does nothing.
static ASKING: AtomicBool = AtomicBool::new(false);
/// The person confirmed (or nothing ran): the exit that follows is not asked about again.
static CONFIRMED: AtomicBool = AtomicBool::new(false);

/// The question's text (heading, then the detail) and its buttons, first to last; the LAST declines and is the
/// default on every OS (`vw_host::dialog_buttons`' rule), so a stray Enter never quits.
pub(crate) fn question(end: End) -> (String, [&'static str; 2]) {
    match end {
        End::Quit => (
            format!("{QUIT_WHILE_WORKING}\n\n{QUIT_DETAIL}"),
            [QUIT, CANCEL],
        ),
        End::Restart => (
            format!("{RESTART_WHILE_WORKING}\n\n{RESTART_DETAIL}"),
            [RESTART, CANCEL],
        ),
    }
}

/// Whether the pressed button (none: the window was closed or failed) confirms `end`.
pub(crate) fn confirms(end: End, pressed: Option<&str>) -> bool {
    pressed == Some(question(end).1[0])
}

/// Agent work runs in an open chat. Before the core is up nothing runs.
fn working(app: &AppHandle) -> bool {
    app.try_state::<vw_host::HostHandle>()
        .and_then(|handle| handle.try_get())
        .is_some_and(|host| {
            host.core()
                .is_some_and(vw_core_api::Core::agent_work_running)
        })
}

/// Ends the app as `end` says, asking the person first while agent work runs.
pub(crate) fn request(app: &AppHandle, end: End) {
    if !working(app) {
        finish(app, end);
        return;
    }
    if ASKING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (text, labels) = question(end);
        let labels = labels.map(str::to_owned);
        let pressed = crate::dialogs::ask_person(&app, text, &labels).await;
        ASKING.store(false, Ordering::Release);
        if confirms(end, pressed.as_deref()) {
            finish(&app, end);
        }
    });
}

fn finish(app: &AppHandle, end: End) {
    CONFIRMED.store(true, Ordering::Release);
    match end {
        End::Quit => app.exit(0),
        End::Restart => crate::tray::restart(app),
    }
}

/// A close of the main window that would end the app (it does not stay in the tray): kept open and asked about while
/// agent work runs. True when the close was held.
pub(crate) fn hold_close(window: &tauri::Window, api: &tauri::CloseRequestApi) -> bool {
    let app = window.app_handle();
    if CONFIRMED.load(Ordering::Acquire) || !working(app) {
        return false;
    }
    api.prevent_close();
    request(app, End::Quit);
    true
}

/// An exit the OS requests for the person (`code` is `None`: macOS Cmd+Q, the app menu's Quit, the last window
/// closed): held and asked about while agent work runs and the main window is still there to return to. The app's own
/// exits (`Some` code: a confirmed Quit, a restart, a failed start) are never held.
pub(crate) fn hold_exit(app: &AppHandle, code: Option<i32>, api: &tauri::ExitRequestApi) {
    if code.is_some()
        || CONFIRMED.load(Ordering::Acquire)
        || crate::main_window(app).is_none()
        || !working(app)
    {
        return;
    }
    api.prevent_exit();
    request(app, End::Quit);
}

#[cfg(test)]
mod tests {
    //! PREDICT (before the change): Quit and Restart ended the app at once, also while an agent worked; there was no
    //! question, so these do not compile.
    use super::{End, confirms, question};

    #[test]
    fn quit_and_restart_name_running_work_and_offer_cancel_as_the_default() {
        let (quit, quit_buttons) = question(End::Quit);
        assert!(
            quit.starts_with(
                "The agent in this chat is running. Are you sure you want to quit?\n\n"
            )
        );
        assert_eq!(quit_buttons, ["Quit", "Cancel"]);
        let (restart, restart_buttons) = question(End::Restart);
        assert!(restart.starts_with(
            "The agent in this chat is running. Are you sure you want to restart?\n\n"
        ));
        assert_eq!(restart_buttons, ["Restart", "Cancel"]);
    }

    #[test]
    fn only_the_named_action_confirms() {
        assert!(confirms(End::Quit, Some("Quit")));
        assert!(confirms(End::Restart, Some("Restart")));
        for pressed in [None, Some("Cancel"), Some("Restart")] {
            assert!(!confirms(End::Quit, pressed), "{pressed:?}");
        }
        assert!(!confirms(End::Restart, Some("Quit")));
        assert!(!confirms(End::Restart, None));
    }
}
