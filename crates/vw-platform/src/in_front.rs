//! Whether one of this app's own windows is in front (the person is looking at the app), for the OS notification rule:
//! a request the person is looking at, also through the app's own confirmation window (a separate window, so the
//! webview reports no focus), is not announced over it.
//!
//! - Windows: the foreground window belongs to this process (`GetForegroundWindow`; the confirmation dialog is ours).
//! - macOS: this application is active (`NSRunningApplication.currentApplication.isActive`; the confirmation is an
//!   `NSAlert` of this app).
//! - Other OSes: `None`; the app asks its toolkit (GTK) on the UI thread instead.
//!
//! Depends on: the `windows` crate (Windows), `objc2-app-kit` (macOS). Called by: the app's `notify_session` command.
//! Also hands Windows foreground permission to an external window requested by core's browser pairing flow.

/// Allows another process to take the foreground for a window this app is about to request. Windows grants this
/// only while this process may set the foreground; other OSes do nothing. Call immediately before the focus request.
#[cfg_attr(
    windows,
    allow(
        unsafe_code,
        reason = "Win32 foreground permission has no safe wrapper"
    )
)]
pub fn allow_foreground_handover() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
        // SAFETY: no pointers or handles are involved. Windows checks the caller's foreground rights and expires
        // this permission on subsequent user input. ASFW_ANY avoids guessing the browser's process from its profile.
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }
}

/// `Some(true)` when one of this process's windows is in front, `Some(false)` when another application's is, `None`
/// when this OS needs the toolkit's answer.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "two read-only Win32 calls without a safe wrapper (unsafe code stays in vw-platform)"
)]
#[must_use]
pub fn own_app_in_front() -> Option<bool> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    // SAFETY: plain reads of the current foreground window and its owning process id.
    let front = unsafe { GetForegroundWindow() };
    if front.0.is_null() {
        return Some(false);
    }
    let mut pid = 0u32;
    // SAFETY: `front` is a window handle just returned by the OS; `pid` is ours to write.
    let _ = unsafe { GetWindowThreadProcessId(front, Some(&raw mut pid)) };
    Some(pid == std::process::id())
}

/// `Some(true)` when this application is the active one, `Some(false)` otherwise.
#[cfg(target_os = "macos")]
#[must_use]
pub fn own_app_in_front() -> Option<bool> {
    Some(objc2_app_kit::NSRunningApplication::currentApplication().isActive())
}

/// This OS needs the toolkit's answer (the app asks GTK on its UI thread).
#[cfg(not(any(windows, target_os = "macos")))]
#[must_use]
pub fn own_app_in_front() -> Option<bool> {
    None
}
