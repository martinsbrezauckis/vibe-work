//! macOS: the current page as a web archive (`WKWebView createWebArchiveData`, macOS 11+), whose bytes hold each
//! frame's document source as the engine received it (and the page's subresources). The built-in browser scans
//! them for closed declarative shadow roots where its private world cannot enter closed roots (macOS before 27).
//! A frame in another web process may be missing from the archive.
//!
//! Depends on: objc2-web-kit `WKWebView`. Called by vw-webview's engine on the main thread.

use std::cell::RefCell;
use std::ffi::c_void;

use block2::RcBlock;
use objc2_foundation::{NSData, NSError};
use objc2_web_kit::WKWebView;

/// Completes once with the archive's bytes or the engine's error text.
pub type ArchiveDone = Box<dyn FnOnce(Result<Vec<u8>, String>) + Send>;

/// Asks `webview` (a `WKWebView` pointer, main thread) for its web archive.
pub fn web_archive(webview: *mut c_void, done: ArchiveDone) {
    // SAFETY: the caller passes the live preview webview on the main thread.
    let Some(view) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        done(Err("the preview webview is not available".to_owned()));
        return;
    };
    let done = RefCell::new(Some(done));
    let block = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
        let Some(done) = done.borrow_mut().take() else {
            return;
        };
        // SAFETY: WebKit's completion arguments, valid for this main-thread call.
        let result = match (unsafe { data.as_ref() }, unsafe { error.as_ref() }) {
            (Some(data), None) => Ok(data.to_vec()),
            (_, Some(error)) => Err(error.localizedDescription().to_string()),
            (None, None) => Err("the page archive is unavailable".to_owned()),
        };
        done(result);
    });
    // SAFETY: a live webview; WebKit calls the block once on the main thread.
    unsafe { view.createWebArchiveDataWithCompletionHandler(&block) };
}
