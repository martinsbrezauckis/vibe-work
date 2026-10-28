//! "Send to Vibe Work" in the system's right-click menus: whether the entries are there, and writing or removing
//! them for the current user. Windows: an Explorer verb for files and folders plus a Send to shortcut
//! (`send_to_windows`). Linux: an application entry, a Nautilus script and a Dolphin service menu
//! (`send_to_linux`). macOS: the app declares two services in its `Info.plist` ("Send to Vibe Work" for files and
//! folders, "Ask Vibe Work" for selected text); the switch turns them on or off in the system's Services list
//! (`send_to_mac`, which also hands their requests to the app).
//!
//! Depends on: std; windows-registry and `windows` (Windows), objc2 (macOS). Called by: `vw-host` (`send_to`).
//! Invariants: nothing is written unless asked ([`install`]); [`remove`] deletes exactly what [`install`] wrote; a
//! failed [`install`] leaves nothing of its own behind.

#[cfg(target_os = "linux")]
#[path = "send_to_linux.rs"]
pub mod linux;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
#[path = "send_to_mac.rs"]
pub mod mac;
#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "send_to_windows.rs"]
pub mod windows;

/// A failure to write or remove an entry, with the plain reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendToError {
    /// The system's menus have no such entries here.
    #[error("this system has no right-click menu entries for apps")]
    Unsupported,
    /// A file, key or setting could not be written or removed.
    #[error("{0}")]
    Write(String),
}

/// What the menus look like now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // independent facts the window shows, not a state machine
pub struct SendToStatus {
    /// The system offers such entries.
    pub supported: bool,
    /// The entries are there (any of them).
    pub on: bool,
    /// Windows 11: the entry is under "Show more options".
    pub under_more_options: bool,
    /// Windows: the installer already asked whether to add the entries (an update installed in the app never asks).
    pub installer_asked: bool,
}

/// Whether the entries are there.
#[must_use]
pub fn status() -> SendToStatus {
    #[cfg(windows)]
    return SendToStatus {
        supported: true,
        on: windows::installed(),
        under_more_options: windows::under_more_options(),
        installer_asked: windows::installer_asked(),
    };
    #[cfg(target_os = "linux")]
    return SendToStatus {
        supported: linux::data_home().is_some(),
        on: linux::data_home().is_some_and(|home| linux::installed_in(&home)),
        under_more_options: false,
        installer_asked: false,
    };
    #[cfg(target_os = "macos")]
    return SendToStatus {
        supported: true,
        on: mac::enabled(),
        under_more_options: false,
        installer_asked: false,
    };
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    SendToStatus {
        supported: false,
        on: false,
        under_more_options: false,
        installer_asked: false,
    }
}

/// Writes the entries for this program; all or nothing.
///
/// # Errors
/// [`SendToError`]: unsupported here, or the first write that failed (nothing of this call is left).
pub fn install() -> Result<(), SendToError> {
    #[cfg(windows)]
    return std::env::current_exe()
        .map_err(|error| SendToError::Write(error.to_string()))
        .and_then(|exe| windows::install(&exe));
    #[cfg(target_os = "linux")]
    return match (linux::data_home(), linux::launcher()) {
        (Some(home), Some(exe)) => linux::install_in(&home, &exe),
        _ => Err(SendToError::Unsupported),
    };
    #[cfg(target_os = "macos")]
    return mac::set_enabled(true);
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    Err(SendToError::Unsupported)
}

/// Removes the entries this app wrote (absent ones are fine).
///
/// # Errors
/// [`SendToError`]: one could not be removed (the others still are).
pub fn remove() -> Result<(), SendToError> {
    #[cfg(windows)]
    return windows::remove();
    #[cfg(target_os = "linux")]
    return linux::data_home().map_or(Ok(()), |home| linux::remove_in(&home));
    #[cfg(target_os = "macos")]
    return mac::set_enabled(false);
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    Ok(())
}
