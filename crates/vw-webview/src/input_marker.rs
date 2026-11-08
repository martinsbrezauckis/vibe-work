//! Best-effort host feedback for input completed by the browser driver. No script or content enters the page.
//! Depends on the native marker adapters; the engine checks mount visibility before calling this on the UI thread.

use std::time::Duration;
use tauri::{Webview, Wry};
use vw_preview::{EngineError, action_tracking::ActionMarker};

pub(crate) async fn show(webview: &Webview<Wry>, marker: ActionMarker) -> Result<(), EngineError> {
    crate::platform::with_native(
        webview,
        "showing browser input",
        Duration::from_secs(1),
        move |native, reply| {
            if reply.is_closed() {
                return;
            }
            let Some(remaining) = marker
                .expires
                .checked_duration_since(std::time::Instant::now())
            else {
                let _ = reply.send(Ok(()));
                return;
            };
            #[cfg(target_os = "linux")]
            let result = {
                use gtk::prelude::*;
                vw_platform::webview::click_marker_linux::show_for(
                    native.inner().upcast_ref(),
                    marker.point,
                    remaining,
                )
            };
            #[cfg(windows)]
            let result = vw_platform::webview::click_marker_win::show_for(
                &native.controller(),
                marker.point,
                remaining,
            );
            #[cfg(target_os = "macos")]
            let result = vw_platform::webview::click_marker_mac::show_for(
                native.inner(),
                marker.point,
                remaining,
            );
            let _ = reply.send(result);
        },
    )
    .await
}
