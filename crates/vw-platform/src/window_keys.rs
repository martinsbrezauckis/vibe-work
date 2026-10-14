//! Keyboard behaviour of the app's main window that the OS decides.
//!
//! Windows: a user's bare Alt tap (press and release with no other key) makes `DefWindowProc` send the window
//! `WM_SYSCOMMAND` / `SC_KEYMENU` with `lParam == 0`, which enters menu mode for the system menu: nothing is shown,
//! but the next key goes to the menu, and previously every bridge call waited until another key or a click (the Debug
//! API's `ui/*` answered 504 after 10 s; Escape released it). The app has no menu bar, so a bare Alt tap has nothing
//! to open: [`ignore_bare_alt`] subclasses the main window and swallows exactly that message. Alt+Space (the system
//! menu, `lParam == ' '`), Alt+F4 and every other `WM_SYSCOMMAND` go on unchanged. Other OSes: nothing to do.
//!
//! Depends on: the `windows` crate (comctl32 `SetWindowSubclass`, also used by wry for the same window). Called by:
//! the app (`app/src/lib.rs`, after the main window is built). Invariant: the subclass procedure only reads its
//! arguments and forwards everything but the one message to `DefSubclassProc`; it holds no state and removes
//! itself when the window is destroyed.

/// Makes a bare Alt tap on the window `hwnd` do nothing (Windows; a no-op elsewhere). `hwnd` is the raw handle of
/// a window created by this process on this thread.
///
/// # Errors
/// A text reason when the window could not be subclassed (Windows); the app then keeps the OS behaviour.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "Win32 window subclassing without a safe wrapper (unsafe code is kept inside vw-platform)"
)]
pub fn ignore_bare_alt(hwnd: isize) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{SC_KEYMENU, WM_NCDESTROY, WM_SYSCOMMAND};

    /// Our id among the window's subclasses (wry uses small numbers for its own).
    const SUBCLASS_ID: usize = 0x5657_414c; // "VWAL"

    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        // The low four bits of wParam are used by the OS (MSDN WM_SYSCOMMAND); lParam 0 = the bare Alt tap.
        if msg == WM_SYSCOMMAND && (wparam.0 & 0xFFF0) == SC_KEYMENU as usize && lparam.0 == 0 {
            return LRESULT(0);
        }
        if msg == WM_NCDESTROY {
            // SAFETY: removing our own subclass from the window being destroyed, on its thread (the documented
            // place to remove a subclass).
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(proc), SUBCLASS_ID) };
        }
        // SAFETY: forwarding the exact arguments the OS gave this subclass procedure, on the window's thread.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    if hwnd == 0 {
        return Err("no window handle".to_owned());
    }
    // SAFETY: `hwnd` is a live window of this process (the caller's contract); `proc` is a plain function that
    // lives for the whole program and keeps no data (`dwRefData` 0); it removes itself at WM_NCDESTROY. Called on
    // the window's own thread (the app's setup runs on the main thread that created it).
    let ok = unsafe { SetWindowSubclass(HWND(hwnd as *mut _), Some(proc), SUBCLASS_ID, 0) };
    if ok.as_bool() {
        Ok(())
    } else {
        Err("SetWindowSubclass failed".to_owned())
    }
}

/// Makes a bare Alt tap on the window do nothing: only Windows has that menu mode, so this does nothing here.
///
/// # Errors
/// Never on this OS.
#[cfg(not(windows))]
pub fn ignore_bare_alt(_hwnd: isize) -> Result<(), String> {
    Ok(())
}
