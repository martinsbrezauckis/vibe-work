//! macOS: a sharp snapshot of a zoomed `WKWebView` (block ): the viewport rendered at one point per CSS pixel
//! (`WKSnapshotConfiguration.snapshotWidth` = the CSS viewport width, [`super::sharp::snapshot_width`]), so a 1280 px
//! Desktop page shown in a 640 pt panel gives a 1280 pt picture (2560 px on a Retina screen) instead of 640 pt.
//! Nothing about the webview changes (no zoom reset, no resize): nothing to restore after a Stop or a timeout.
//!
//! Depends on: objc2, objc2-foundation (`NSNumber`), objc2-web-kit (`WKSnapshotConfiguration`), objc2-app-kit
//! (`NSWindow` backing scale), block2, `super::png_of` encoding. Called by: vw-webview `platform::screenshot_sharp`
//! (inside Tauri's `with_webview`, on the main thread).
//! Invariants:
//! - `snapshot_png_sharp` answers its callback exactly once (also on failure); the pixel limits are checked before
//!   `WebKit` allocates anything.
//! - `unsafe` is confined to the platform calls, each with its SAFETY comment.

use std::ffi::c_void;
use std::sync::{Mutex, PoisonError};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::NSImage;
use objc2_foundation::{NSError, NSNumber};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

use super::PngDone;

/// Takes a PNG of `webview` (Tauri's `PlatformWebview::inner()`, a `WKWebView`, on the main thread) rendered at one
/// point per CSS pixel; at page zoom 1 or more an ordinary snapshot. Answers `done` when `WebKit` hands over the
/// image, or with the reason (invalid geometry, the pixel limits, `WebKit`'s error).
pub fn snapshot_png_sharp(webview: *mut c_void, done: PngDone) {
    let Some(mtm) = MainThreadMarker::new() else {
        done(Err("a snapshot must be taken on the main thread".to_owned()));
        return;
    };
    if webview.is_null() {
        done(Err("the preview webview is not available".to_owned()));
        return;
    }
    // SAFETY: Tauri hands us its live `WKWebView` pointer on the main thread inside `with_webview`; it stays alive for
    // this call (the snapshot request retains what it needs).
    let view = unsafe { &*webview.cast::<WKWebView>() };
    let bounds = view.bounds().size;
    // SAFETY: plain property reads on the main thread.
    let zoom = unsafe { view.pageZoom() };
    let scale = view
        .window()
        .map_or(1.0, |window| window.backingScaleFactor());
    let width = match super::sharp::snapshot_width((bounds.width, bounds.height), zoom, scale) {
        Ok(width) => width,
        Err(reason) => {
            done(Err(reason));
            return;
        }
    };
    let done = Mutex::new(Some(done));
    let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() else {
            return;
        };
        // SAFETY: WebKit passes either a valid image or a valid error for the duration of this call.
        let result = match unsafe { image.as_ref() } {
            Some(image) => super::macos_impl::png_of(image),
            None => Err(unsafe { error.as_ref() }.map_or_else(
                || "snapshot failed".to_owned(),
                |e| e.localizedDescription().to_string(),
            )),
        };
        done(result);
    });
    // SAFETY: a fresh configuration on the main thread; WebKit copies it and retains the block until it calls it.
    unsafe {
        let config = width.map(|width| {
            let config = WKSnapshotConfiguration::new(mtm);
            config.setSnapshotWidth(Some(&NSNumber::new_f64(width)));
            config
        });
        view.takeSnapshotWithConfiguration_completionHandler(config.as_deref(), &block);
    }
}
