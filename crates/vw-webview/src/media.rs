//! Media capture in the main window: what the engine itself must allow before the
//! main window's permission handler (vw-host `media`) is ever asked.
//!
//! - **Linux (WebKitGTK):** `navigator.mediaDevices` and `getUserMedia` exist only when the webview's
//!   `enable-media-stream` setting is on (wry never sets it). `WebKitGTK` 2.52.6
//!   creates new settings with it ON, and the real app recorded with or without this call; older `WebKitGTK`
//!   releases created it OFF. So on current systems the permission handler (vw-host `media`) is what dictation
//!   needed, and this call keeps it working on an older `WebKitGTK`. `WebKit` decides whether the page gets
//!   `navigator.mediaDevices` when it builds the page's script globals, so the setting must be on before the
//!   first document of the main window is created: [`enable_media_capture`] runs synchronously right after the
//!   window is built (setup runs on the main thread, where Tauri's `with_webview` runs the closure at once; wry
//!   has only queued the first load by then). If a document had already finished loading, the webview is
//!   reloaded once so the new setting takes effect.
//!   `enable-webrtc` stays off (its default): dictation needs capture only, no peer connections.
//! - **Windows and macOS:** nothing to switch on; `WebView2` and `WKWebView` expose `getUserMedia` on the app's
//!   secure origin and ask the permission handler.
//!
//! Depends on: tauri (`with_webview`), and on Linux webkit2gtk. Called by: the app (`app/src/lib.rs`), for the main
//! window only. Child webviews never get this (their permission handlers refuse everything anyway).

use tauri::{WebviewWindow, Wry};

/// Turns on what the main window's engine needs for `getUserMedia` (Linux: `WebKitGTK` `enable-media-stream`; other
/// systems: nothing).
///
/// # Errors
/// Tauri's error when the webview is gone (the closure could not be scheduled).
pub fn enable_media_capture(window: &WebviewWindow<Wry>) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    {
        window.with_webview(|platform| {
            use webkit2gtk::{SettingsExt as _, WebViewExt};
            let view = platform.inner();
            let Some(settings) = WebViewExt::settings(&view) else {
                tracing::warn!(
                    "the main webview has no settings: dictation cannot open the microphone"
                );
                return;
            };
            if settings.enables_media_stream() {
                return;
            }
            settings.set_enable_media_stream(true);
            // A document that already finished loading kept its script globals without `mediaDevices`.
            let loaded = !view.is_loading() && view.uri().is_some_and(|uri| !uri.is_empty());
            if loaded {
                tracing::info!(
                    "media stream turned on after the first load: reloading the main window once"
                );
                view.reload();
            }
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        Ok(())
    }
}
