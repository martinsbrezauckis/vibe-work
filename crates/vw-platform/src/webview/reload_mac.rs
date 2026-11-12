//! Native cache-bypass reload of one `WKWebView`. Called on the UI thread by vw-webview.
//! Uses `WebKit`'s `reloadFromOrigin`; no profile cache or other page is changed.
use objc2_web_kit::WKWebView;

/// Starts revalidation from the origin in the supplied live webview.
///
/// # Errors
/// The view is unavailable or `WebKit` declined to create a navigation.
pub fn reload_from_origin(view: *mut std::ffi::c_void) -> Result<(), String> {
    // SAFETY: Tauri supplies a live WKWebView on its UI thread for this synchronous call.
    let view = unsafe { view.cast::<WKWebView>().as_ref() }
        .ok_or("Browser view is unavailable; reopen the tab.")?;
    // SAFETY: the receiver is the live UI-thread view; WebKit owns the returned navigation.
    unsafe { view.reloadFromOrigin() }
        .map(|_| ())
        .ok_or_else(|| "Reload did not start; check the browser page before retrying.".into())
}
