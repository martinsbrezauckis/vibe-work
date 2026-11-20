//! Ending this process at once, for an app whose UI thread is stuck: exit handlers could wait for that thread, so on
//! Unix none run (`_exit`). Windows ends through `std::process::exit`.
//!
//! Depends on: libc (Unix). Called by: the app's start watch.

/// Ends the process now with `code`, without running exit handlers (Unix).
pub fn now(code: i32) -> ! {
    #[cfg(unix)]
    // SAFETY: `_exit` takes any status and does not return; it runs no Rust or C code of this process afterwards.
    unsafe {
        libc::_exit(code)
    }
    #[cfg(not(unix))]
    std::process::exit(code)
}
