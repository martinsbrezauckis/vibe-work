//! VIBE WORK PATCH: WK frame identity during the synchronous navigation callback.
//! Depends only on thread-local state; read by vw-webview through Tauri's URL-only callback.
//! Unknown context fails closed for top-level continuation; nested callbacks restore their caller's context.

use std::cell::Cell;

thread_local! {
  static MAIN_FRAME: Cell<Option<bool>> = const { Cell::new(None) };
}

/// True only inside a WK main-frame navigation callback.
#[must_use]
pub fn is_main_frame() -> bool {
    MAIN_FRAME.get() == Some(true)
}

#[cfg(any(target_os = "macos", target_os = "ios", test))]
pub(crate) fn with_main_frame<T>(main: bool, callback: impl FnOnce() -> T) -> T {
    struct Restore(Option<bool>);
    impl Drop for Restore {
        fn drop(&mut self) {
            MAIN_FRAME.set(self.0);
        }
    }
    let _restore = Restore(MAIN_FRAME.replace(Some(main)));
    callback()
}

#[cfg(test)]
mod tests {
    use super::{is_main_frame, with_main_frame};

    #[test]
    fn nested_frame_context_restores_main_frame_and_then_unknown() {
        assert!(!is_main_frame());
        with_main_frame(true, || {
            assert!(is_main_frame());
            with_main_frame(false, || assert!(!is_main_frame()));
            assert!(is_main_frame());
        });
        assert!(!is_main_frame());
    }
}
