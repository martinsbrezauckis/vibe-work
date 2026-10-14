//! OS notifications that can report a click.
//!
//! Depends on: `notify-rust` (Linux: freedesktop notifications over D-Bus through pure-Rust zbus;
//! macOS: the user notification centre; Windows: toast with an activation handler). Called by:
//! vw-host (`Host::notify`), through the [`Notifier`] trait so tests use a recording fake.
//! Why not the Tauri notification plugin: its desktop `show()` builds a `notify_rust::Notification`
//! and drops the handle, so a click can never be observed (`tauri-plugin-notification` 2.5.1
//! `desktop.rs`: "Only the title, body, icon and sound ... are used on desktop"). This module does
//! what the plugin does (Windows app id only for an installed app, macOS application id) and keeps
//! the handle.
//! Invariants:
//! - `show` returns as soon as the notification is handed to the OS; a click is reported later,
//!   once, on a helper thread.
//! - Linux/macOS: at most [`MAX_WAITERS`] helper threads wait for clicks at once; beyond that the
//!   notification is shown without click handling (never blocks, never grows without bound).
//!   Windows needs no thread (the toast's handlers run on the OS's callback thread).
//! - Text is passed through as plain text; the OS decides how to render it.
//! - Linux requests the packaged `vibe-work` hicolor icon; Windows/macOS use the installed app identity.
//! - A click is the body or the "Open" button on every platform. Windows uses the toast library
//!   underneath notify-rust directly ([`show_toast`]): notify-rust's handle stops listening at the
//!   first event, and Windows reports "timed out" when a toast moves to the Notification Center.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
#[cfg(not(windows))]
use std::sync::atomic::Ordering;

/// Most notifications whose click we are still waiting for. A server closes a notification after
/// its own timeout, which ends the waiting thread.
#[cfg(not(windows))]
const MAX_WAITERS: usize = 16;

/// Why a notification could not be shown.
#[derive(Debug, thiserror::Error)]
#[error("the notification could not be shown: {0}")]
pub struct NotifyError(String);

/// What to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRequest {
    /// Title line.
    pub title: String,
    /// Body text.
    pub body: String,
}

/// Shows an OS notification and calls `on_click` when the user activates it.
pub trait Notifier: Send + Sync {
    /// Shows `request`. `on_click` runs at most once, on another thread, only for a click on the
    /// notification itself (not for dismissal or expiry).
    ///
    /// # Errors
    /// [`NotifyError`] when the OS refuses or has no notification service.
    fn show(
        &self,
        request: NotificationRequest,
        on_click: Box<dyn FnOnce() + Send>,
    ) -> Result<(), NotifyError>;
}

/// True when a Windows toast activation is the user opening the notification: a click on its body
/// (no arguments: the toast has no `launch` value) or on its "Open" button (`"default"`, the action
/// this module adds). Any other button is not.
#[cfg(any(windows, test))]
#[must_use]
pub(crate) fn is_open_activation(arguments: Option<&str>) -> bool {
    matches!(arguments, None | Some("default"))
}

/// Windows: the toast with persistent activation handling. notify-rust's handle reports only the
/// FIRST event and Windows sends "dismissed: timed out" when the banner leaves the screen (or at once
/// when banners are off), while the toast stays in the Notification Center and can still be clicked:
/// observed on Windows 11 (`notify_probe --raw`: `Closed(Expired)` before any click, every click in
/// the Notification Center lost). Its body click also reached `wait_for_action` as "__closed"
/// (notify-rust 4.18.2). The toast's own `Activated` handler fires for every activation while the
/// process lives; `on_click` still runs at most once.
#[cfg(windows)]
fn show_toast(
    app_id: &str,
    request: &NotificationRequest,
    on_click: Box<dyn FnOnce() + Send>,
) -> Result<(), NotifyError> {
    use tauri_winrt_notification::{Duration, Toast};
    let once = std::sync::Mutex::new(Some(on_click));
    Toast::new(app_id)
        .title(&request.title)
        .text1(&request.body)
        .sound(None)
        .duration(Duration::Short)
        .add_button("Open", "default")
        .on_activated(move |arguments| {
            if is_open_activation(arguments.as_deref()) {
                let pending = once
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let Some(click) = pending {
                    click();
                }
            }
            Ok(())
        })
        .show()
        .map_err(|error| NotifyError(error.to_string()))
}

/// The real notifier.
#[derive(Debug, Clone)]
pub struct DesktopNotifier {
    app_id: String,
    installed: bool,
    waiters: Arc<AtomicUsize>,
}

impl DesktopNotifier {
    /// `app_id` is the bundle identifier. `installed` is false for development runs, where Windows
    /// has no registered app identity and macOS borrows the terminal's.
    #[must_use]
    pub fn new(app_id: impl Into<String>, installed: bool) -> Self {
        Self {
            app_id: app_id.into(),
            installed,
            waiters: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Notifier for DesktopNotifier {
    fn show(
        &self,
        request: NotificationRequest,
        on_click: Box<dyn FnOnce() + Send>,
    ) -> Result<(), NotifyError> {
        #[cfg(windows)]
        {
            let _ = &self.waiters;
            let app_id = if self.installed {
                self.app_id.as_str()
            } else {
                tauri_winrt_notification::Toast::POWERSHELL_APP_ID
            };
            show_toast(app_id, &request, on_click)
        }
        #[cfg(not(windows))]
        self.show_and_wait(&request, on_click)
    }
}

impl DesktopNotifier {
    /// Linux and macOS: notify-rust, and one helper thread waiting for the click (the server's
    /// close ends it).
    #[cfg(not(windows))]
    fn show_and_wait(
        &self,
        request: &NotificationRequest,
        on_click: Box<dyn FnOnce() + Send>,
    ) -> Result<(), NotifyError> {
        let mut notification = notify_rust::Notification::new();
        notification
            .summary(&request.title)
            .body(&request.body)
            .action("default", "Open");
        // Tauri installs bundle PNGs under mainBinaryName from app/tauri.conf.json in hicolor.
        #[cfg(target_os = "linux")]
        notification.icon("vibe-work");
        #[cfg(target_os = "macos")]
        {
            let _ = notify_rust::set_application(if self.installed {
                self.app_id.as_str()
            } else {
                "com.apple.Terminal"
            });
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (&self.app_id, self.installed);
        let handle = notification
            .show()
            .map_err(|error| NotifyError(error.to_string()))?;
        if self.waiters.fetch_add(1, Ordering::AcqRel) >= MAX_WAITERS {
            self.waiters.fetch_sub(1, Ordering::AcqRel);
            return Ok(());
        }
        let waiters = self.waiters.clone();
        let spawned = std::thread::Builder::new()
            .name("vw-notification-click".to_owned())
            .spawn(move || {
                handle.wait_for_action(|action| {
                    if action == "default" {
                        on_click();
                    }
                });
                waiters.fetch_sub(1, Ordering::AcqRel);
            });
        if spawned.is_err() {
            self.waiters.fetch_sub(1, Ordering::AcqRel);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::is_open_activation;

    /// A body click arrives with no arguments, the "Open" button with "default"; both open the
    /// session. Another button does not.
    #[test]
    fn body_and_open_button_are_clicks_other_buttons_are_not() {
        assert!(is_open_activation(None));
        assert!(is_open_activation(Some("default")));
        assert!(!is_open_activation(Some("other")));
    }
}
