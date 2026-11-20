//! The native title bar's look: dark or light to match the app's own theme. On Windows the title bar is drawn by the
//! desktop window manager, which keeps it white for a light system theme even while the app shows its dark theme;
//! its immersive dark mode attribute switches it. Only the title bar changes: the web view's `prefers-color-scheme`
//! keeps following the system (the app's "follow the system theme" reads it). Other systems are left as they are.
//! Depends on: the `windows` crate (DWM). Called by: the app's title bar bridge command.

/// Draws the title bar of the top-level window `hwnd` dark (`true`) or light.
///
/// # Errors
/// The window manager's error when it refuses the attribute (a Windows build without dark title bars).
#[cfg(windows)]
#[expect(
    unsafe_code,
    reason = "DwmSetWindowAttribute has no safe wrapper (unsafe code is kept inside vw-platform)"
)]
pub fn set_title_bar_dark(hwnd: isize, dark: bool) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE, DwmSetWindowAttribute,
    };
    use windows::core::BOOL;

    /// The attribute's number before Windows 10 20H1 (build 19041), where 20 is unknown.
    const BEFORE_20H1: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(19);
    let value = BOOL::from(dark);
    let size = u32::try_from(std::mem::size_of::<BOOL>()).map_err(|e| e.to_string())?;
    let hwnd = HWND(hwnd as *mut _);
    let set = |attribute: DWMWINDOWATTRIBUTE| {
        // SAFETY: `value` is a live BOOL of `size` bytes for the whole call, as the attribute requires; the window
        // manager copies it and only reads it.
        unsafe { DwmSetWindowAttribute(hwnd, attribute, std::ptr::from_ref(&value).cast(), size) }
    };
    set(DWMWA_USE_IMMERSIVE_DARK_MODE)
        .or_else(|_| set(BEFORE_20H1))
        .map_err(|e| e.to_string())
}

/// Leaves the title bar as the system draws it (no app-owned title bar colour off Windows).
///
/// # Errors
/// Never on this OS.
#[cfg(not(windows))]
pub fn set_title_bar_dark(_hwnd: isize, _dark: bool) -> Result<(), String> {
    Ok(())
}
