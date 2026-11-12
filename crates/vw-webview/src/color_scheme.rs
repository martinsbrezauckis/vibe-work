//! Per-view native media/appearance override, called by the preview engine on the UI thread.
//! Does not change app-wide GTK settings or patch the page's media-query implementation.
use tauri::{Webview, Wry};
use vw_preview::EngineError;

#[cfg(target_os = "linux")]
#[allow(clippy::unused_async)]
pub(crate) async fn set(_webview: &Webview<Wry>, dark: Option<bool>) -> Result<(), EngineError> {
    if dark.is_none() {
        Ok(())
    } else {
        Err(EngineError::Unsupported("WebKitGTK has no per-view color preference API; use the system appearance or a browser with native media emulation.".into()))
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn set(webview: &Webview<Wry>, dark: Option<bool>) -> Result<(), EngineError> {
    crate::platform::with_native(webview, "changing page color", std::time::Duration::from_secs(5), move |native, reply| {
        #[cfg(target_os = "macos")]
        { let _ = reply.send(vw_platform::webview::set_color_scheme(native.inner(), dark)); }
        #[cfg(windows)]
        match vw_platform::webview::devtools_win::DevTools::new(&native.controller()) {
            Ok(channel) => {
                let features: Vec<_> = dark.into_iter().map(|dark| serde_json::json!({"name":"prefers-color-scheme","value":if dark {"dark"} else {"light"}})).collect();
                channel.call("", "Emulation.setEmulatedMedia", &serde_json::json!({"features":features}).to_string(), Box::new(move |result| {let _ = reply.send(result.map(|_| ()));}));
            }
            Err(error) => { let _ = reply.send(Err(error)); }
        }
    }).await
}
