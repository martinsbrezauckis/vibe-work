//! Windows: keeps tao's per-thread event-target window alive when something closes the app "politely".
//!
//! tao gives the UI thread a top-level "Tao Thread Event Target" window (visible, unowned, transparent) through which
//! the event-loop proxy delivers. Its window procedure leaves `WM_CLOSE` to `DefWindowProc`, which destroys it. While
//! the main window is minimized this window is the process's only visible unowned top-level window, so
//! `taskkill /PID` without `/F` and scripts that close `Process.MainWindowHandle` send their `WM_CLOSE` to it: the event
//! loop loses its message route and the app keeps running with a wedged main window.
//! [`keep_event_target_on_close`] subclasses that window so a `WM_CLOSE` there is passed on to the main window
//! instead of destroying it: a polite close of the minimized app closes it like the window's X button, and the
//! event loop keeps its route. Other OSes: nothing to do.
//!
//! Depends on: the `windows` crate (user32 `EnumThreadWindows`, comctl32 `SetWindowSubclass`). Called by: the app
//! (`app/src/lib.rs`, on the UI thread after the main window is built). Invariant: the subclass procedure's only
//! data is the main window's handle (`dwRefData`); it changes only `WM_CLOSE` (posted on to the main window, never
//! destroying the event target) and removes itself when the window is destroyed. Can go once tao handles `WM_CLOSE`
//! in its event-target procedure.

/// tao's class name for the per-thread event-target window (tao `platform_impl/windows/event_loop.rs`).
#[cfg(windows)]
const EVENT_TARGET_CLASS: &str = "Tao Thread Event Target";

/// Passes a `WM_CLOSE` sent to this thread's tao event-target window(s) on to the main window `main_hwnd` instead of
/// destroying the event target (Windows; a no-op elsewhere). Call it on the UI thread after the main window is
/// built. Returns how many windows were subclassed (0 when tao made none here).
///
/// # Errors
/// A text reason when a found window could not be subclassed; the app then keeps the OS behaviour.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "Win32 window enumeration and subclassing without a safe wrapper (unsafe code is kept inside vw-platform)"
)]
pub fn keep_event_target_on_close(main_hwnd: isize) -> Result<usize, String> {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetClassNameW, PostMessageW, WM_CLOSE, WM_NCDESTROY,
    };
    use windows::core::BOOL;

    /// Our id among the window's subclasses.
    const SUBCLASS_ID: usize = 0x5657_4554; // "VWET"

    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        main: usize,
    ) -> LRESULT {
        if msg == WM_CLOSE {
            // Closing the app is the main window's job (its CloseRequested path); this window must stay.
            // SAFETY: posting to a window handle; a stale handle only makes the post fail.
            let _ =
                unsafe { PostMessageW(Some(HWND(main as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            return LRESULT(0);
        }
        if msg == WM_NCDESTROY {
            // SAFETY: removing our own subclass from the window being destroyed, on its thread.
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(proc), SUBCLASS_ID) };
        }
        // SAFETY: forwarding the exact arguments the OS gave this subclass procedure, on the window's thread.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    unsafe extern "system" fn collect(hwnd: HWND, found: LPARAM) -> BOOL {
        let mut name = [0u16; 64];
        // SAFETY: `hwnd` comes from EnumThreadWindows; the buffer is ours and its length is passed.
        let len = unsafe { GetClassNameW(hwnd, &mut name) };
        let len = usize::try_from(len).unwrap_or(0);
        if String::from_utf16_lossy(&name[..len]) == EVENT_TARGET_CLASS {
            // SAFETY: `found` is the address of the Vec below, alive for the whole synchronous enumeration.
            let list = unsafe { &mut *(found.0 as *mut Vec<HWND>) };
            list.push(hwnd);
        }
        BOOL(1)
    }

    if main_hwnd == 0 {
        return Err("no main window handle".to_owned());
    }
    let mut found: Vec<HWND> = Vec::new();
    // SAFETY: synchronous enumeration of this thread's top-level windows; the callback only reads class names and
    // pushes into `found`, which outlives the call.
    let _ = unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(collect),
            LPARAM(&raw mut found as isize),
        )
    };
    for hwnd in &found {
        // SAFETY: a live window of this thread; `proc` keeps only the main window's handle and removes itself at
        // WM_NCDESTROY.
        let ok =
            unsafe { SetWindowSubclass(*hwnd, Some(proc), SUBCLASS_ID, main_hwnd.cast_unsigned()) };
        if !ok.as_bool() {
            return Err("SetWindowSubclass failed on the event-target window".to_owned());
        }
    }
    Ok(found.len())
}

/// Only Windows has tao's event-target window: nothing to do here.
///
/// # Errors
/// Never on this OS.
#[cfg(not(windows))]
pub fn keep_event_target_on_close(_main_hwnd: isize) -> Result<usize, String> {
    Ok(0)
}
