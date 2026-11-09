//! Captures each marker API's own last error before formatting, cleanup or another native call.
//! Used only by the Windows renderer; clears ignored class-registration errors before the next call.

use windows::Win32::Foundation::{ERROR_SUCCESS, GetLastError, SetLastError, WIN32_ERROR};

pub(super) fn win32<T>(
    call: &str,
    run: impl FnOnce() -> windows::core::Result<T>,
) -> Result<T, String> {
    // SAFETY: last-error state belongs to this UI thread; the closure makes exactly one native call.
    unsafe { SetLastError(ERROR_SUCCESS) };
    let result = run();
    // SAFETY: read immediately, before formatting or cleanup can replace the failing call's error.
    let code = unsafe { GetLastError() };
    result.map_err(|_| error_text(call, code))
}

pub(super) fn error_text(call: &str, code: WIN32_ERROR) -> String {
    let reason = if code == ERROR_SUCCESS {
        "the call did not supply an error code".into()
    } else {
        std::io::Error::from_raw_os_error(i32::from_ne_bytes(code.0.to_ne_bytes())).to_string()
    };
    format!(
        "{call} failed (GetLastError={}, 0x{:08x}): {reason}",
        code.0, code.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::{
            Foundation::{ERROR_ACCESS_DENIED, ERROR_CLASS_ALREADY_EXISTS},
            UI::WindowsAndMessaging::{CreateWindowExW, WINDOW_EX_STYLE, WS_POPUP},
        },
        core::w,
    };

    #[test]
    fn click_marker_error_captures_the_failing_call_before_cleanup() {
        // SAFETY: this test only changes its own thread's error state.
        unsafe { SetLastError(ERROR_CLASS_ALREADY_EXISTS) };
        let error = win32("UpdateLayeredWindow", || {
            // SAFETY: simulates a native failure with a documented last-error code on this thread.
            unsafe { SetLastError(ERROR_ACCESS_DENIED) };
            Err::<(), _>(windows::core::Error::empty())
        })
        .unwrap_err();
        // SAFETY: cleanup may change last-error state; the diagnostic must already own its code.
        unsafe { SetLastError(ERROR_CLASS_ALREADY_EXISTS) };
        assert!(error.contains("UpdateLayeredWindow failed (GetLastError=5, 0x00000005)"));
        assert!(!error.contains("1410"));
    }

    #[test]
    fn click_marker_error_without_a_code_does_not_reuse_class_registration_error() {
        // SAFETY: this test only changes its own thread's error state.
        unsafe { SetLastError(ERROR_CLASS_ALREADY_EXISTS) };
        let error = win32("CreateWindowExW", || {
            Err::<(), _>(windows::core::Error::empty())
        })
        .unwrap_err();
        assert!(error.contains("CreateWindowExW failed (GetLastError=0, 0x00000000)"));
        assert!(error.contains("did not supply an error code"));
        assert!(!error.contains("Class already exists"));
    }

    #[test]
    fn click_marker_error_from_a_real_create_window_failure_names_the_call() {
        let error = win32("CreateWindowExW", || {
            // SAFETY: an unregistered class fails before creating a window; no desktop input is sent.
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("UnregisteredClickMarkerClass"),
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    28,
                    28,
                    None,
                    None,
                    None,
                    None,
                )
            }
        })
        .unwrap_err();
        assert!(error.contains("CreateWindowExW failed (GetLastError=1407, 0x0000057f)"));
    }
}
