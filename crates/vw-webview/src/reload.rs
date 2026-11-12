//! Cache-bypassing reload through the live native engine, called by browser history dispatch.
//! Depends on the UI-thread bridge; the driver waits for navigation and reads the resulting page.
use std::time::Duration;
use tauri::{Webview, Wry};
use vw_preview::EngineError;

pub(crate) async fn bypass_cache(webview: &Webview<Wry>) -> Result<(), EngineError> {
    crate::platform::with_native(
        webview,
        "reloading without cache",
        Duration::from_secs(5),
        |native, reply| {
            #[cfg(target_os = "linux")]
            {
                use webkit2gtk::WebViewExt as _;
                native.inner().reload_bypass_cache();
                let _ = reply.send(Ok(()));
            }
            #[cfg(target_os = "macos")]
            {
                let _ = reply.send(vw_platform::webview::reload_from_origin(native.inner()));
            }
            #[cfg(windows)]
            match vw_platform::webview::devtools_win::DevTools::new(&native.controller()) {
                Ok(channel) => channel.call(
                    "",
                    "Page.reload",
                    r#"{"ignoreCache":true}"#,
                    Box::new(move |result| {
                        let _ = reply.send(result.map(|_| ()));
                    }),
                ),
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            }
        },
    )
    .await
}
