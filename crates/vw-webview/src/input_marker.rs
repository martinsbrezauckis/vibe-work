//! Best-effort host feedback for input completed by the browser driver. No script or content enters the page.
//! Depends on the native marker adapters; the engine checks mount visibility before calling this on the UI thread.

use std::time::Duration;
use tauri::{Webview, Wry};
use vw_preview::{EngineError, NativeClickPoint};

pub(crate) async fn show(
    webview: &Webview<Wry>,
    point: NativeClickPoint,
) -> Result<(), EngineError> {
    let point = vw_platform::native_click::normalized(
        (point.x, point.y),
        (point.viewport_width, point.viewport_height),
    )
    .map_err(EngineError::Platform)?;
    crate::platform::with_native(
        webview,
        "showing browser input",
        Duration::from_secs(1),
        move |native, reply| {
            if reply.is_closed() {
                return;
            }
            #[cfg(target_os = "linux")]
            let result = {
                use gtk::prelude::*;
                vw_platform::webview::click_marker_linux::show(native.inner().upcast_ref(), point)
            };
            #[cfg(windows)]
            let result = vw_platform::webview::click_marker_win::show(&native.controller(), point);
            #[cfg(target_os = "macos")]
            let result = vw_platform::webview::click_marker_mac::show(native.inner(), point);
            let _ = reply.send(result);
        },
    )
    .await
}
