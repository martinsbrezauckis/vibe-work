//! Screen-reader activity for automatic prompt sounds. Called by the speech API; unknown stays unknown.

/// Read the system's screen-reader flag without starting a reader or requesting accessibility permission.
#[must_use]
pub fn active() -> Option<bool> {
    platform_active()
}

#[cfg(target_os = "linux")]
fn platform_active() -> Option<bool> {
    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    // Never autolaunch a desktop bus from a headless process.
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS")?;
    let connection = dbus::blocking::Connection::new_session().ok()?;
    connection
        .with_proxy(
            "org.a11y.Bus",
            "/org/a11y/bus",
            std::time::Duration::from_millis(300),
        )
        .get("org.a11y.Status", "ScreenReaderEnabled")
        .ok()
}

#[cfg(target_os = "macos")]
#[allow(clippy::unnecessary_wraps)] // one signature for every platform; the others can be unknown
fn platform_active() -> Option<bool> {
    Some(objc2_app_kit::NSWorkspace::sharedWorkspace().isVoiceOverEnabled())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn platform_active() -> Option<bool> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETSCREENREADER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut enabled = windows::core::BOOL(0);
    // SAFETY: SPI_GETSCREENREADER writes one BOOL; the pointer remains valid for the synchronous call.
    unsafe {
        SystemParametersInfoW(
            SPI_GETSCREENREADER,
            0,
            Some((&raw mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()
    .map(|()| enabled.as_bool())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_active() -> Option<bool> {
    None
}
