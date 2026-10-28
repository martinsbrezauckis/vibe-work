//! Removing a webview's service-worker registrations: a worker a page registered (also through
//! another realm's `register`, which the page environment cannot refuse) outlives the page and could send requests
//! under a later mount of the same storage. The preview engine calls this at every guarded web mount, before the
//! first page, on every platform (Linux: `WebKitGTK`'s website data manager, in vw-webview).
//!
//! - **Windows:** `ICoreWebView2Profile2::ClearBrowsingData(COREWEBVIEW2_BROWSING_DATA_KINDS_SERVICE_WORKERS)` on the
//!   webview's profile (the mount's storage profile).
//! - **macOS:** `WKWebsiteDataStore removeDataOfTypes:[WKWebsiteDataTypeServiceWorkerRegistrations]
//!   modifiedSince:distantPast` on the webview's data store.
//!
//! Depends on: `webview2-com` + windows (Windows), objc2 / objc2-web-kit / block2 (macOS). Called by: vw-webview
//! `platform::clear_service_workers` (inside Tauri's `with_webview`, on the UI thread).
//! Invariant: `done` is called exactly once, also when the call cannot start.

/// Receives the result of a clearing (a text reason on failure).
pub type ClearDone = Box<dyn FnOnce(Result<(), String>) + Send>;

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::sync::{Arc, Mutex, PoisonError};

    use webview2_com::ClearBrowsingDataCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_BROWSING_DATA_KINDS_SERVICE_WORKERS, ICoreWebView2_13,
        ICoreWebView2Controller, ICoreWebView2Profile2,
    };
    use windows::core::Interface;

    use super::ClearDone;

    /// Clears the service-worker registrations of the profile of `controller`'s webview.
    pub fn clear_service_workers(controller: &ICoreWebView2Controller, done: ClearDone) {
        let done = Arc::new(Mutex::new(Some(done)));
        let finish = {
            let done = done.clone();
            move |result: Result<(), String>| {
                if let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    done(result);
                }
            }
        };
        let callback = finish.clone();
        let handler = ClearBrowsingDataCompletedHandler::create(Box::new(move |status| {
            callback(status.map_err(|e| format!("clearing service workers failed: {e}")));
            Ok(())
        }));
        // SAFETY: the live controller is supplied by Tauri on the UI thread; the profile interfaces exist in every
        // Evergreen runtime since 2022 (a cast failure is reported, never assumed). The handler is called once on the UI
        // thread.
        let started = unsafe {
            controller
                .CoreWebView2()
                .and_then(|core| core.cast::<ICoreWebView2_13>())
                .and_then(|core| core.Profile())
                .and_then(|profile| profile.cast::<ICoreWebView2Profile2>())
                .and_then(|profile| {
                    profile.ClearBrowsingData(
                        COREWEBVIEW2_BROWSING_DATA_KINDS_SERVICE_WORKERS,
                        &handler,
                    )
                })
        };
        if let Err(error) = started {
            finish(Err(format!(
                "clearing service workers could not start: {error}"
            )));
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Mutex;

    use block2::RcBlock;
    use objc2_foundation::{NSDate, NSSet};
    use objc2_web_kit::{WKWebView, WKWebsiteDataTypeServiceWorkerRegistrations};

    use super::ClearDone;

    /// Clears the service-worker registrations of `webview`'s (a `WKWebView`) website data store.
    pub fn clear_service_workers(webview: *mut c_void, done: ClearDone) {
        if webview.is_null() {
            done(Err("the preview webview is not available".to_owned()));
            return;
        }
        let done = Mutex::new(Some(done));
        let block = RcBlock::new(move || {
            if let Some(done) = done
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                done(Ok(()));
            }
        });
        // SAFETY: Tauri hands over its live `WKWebView` on the main thread inside `with_webview`; the type constant is
        // a WebKit framework string; WebKit retains the completion block until it calls it (once).
        unsafe {
            let view = &*webview.cast::<WKWebView>();
            let types = NSSet::from_slice(&[WKWebsiteDataTypeServiceWorkerRegistrations]);
            view.configuration()
                .websiteDataStore()
                .removeDataOfTypes_modifiedSince_completionHandler(
                    &types,
                    &NSDate::distantPast(),
                    &block,
                );
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub use imp::clear_service_workers;
