//! The main window's page process ending under it (a crash, the system ending it, or no answer any more): reported to
//! the app, which reloads the page and tells the person.
//! Linux: `WebKitGTK` `web-process-terminated`; Windows: `WebView2` `ProcessFailed` (vw-platform); macOS: the app's Tauri
//! builder hook (`on_web_content_process_terminate`), so nothing is registered here.
//! Depends on: tauri (`with_webview`), webkit2gtk, vw-platform. Called by: the app's window setup.
use std::sync::Arc;
use tauri::{WebviewWindow, Wry};

/// What happened to the page's process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageCrash {
    /// It ended (crashed, killed, or out of memory): a reload starts a new one.
    Ended,
    /// It stopped answering: a reload replaces it.
    Unresponsive,
    /// The whole web engine ended: the window cannot be reloaded, only the app restarted.
    EngineEnded,
}

/// Calls `handler` on the UI thread whenever the page process of `window`'s webview ends ([`PageCrash`]).
///
/// # Errors
/// The native webview could not be reached.
pub fn on_page_crash(
    window: &WebviewWindow<Wry>,
    handler: Arc<dyn Fn(PageCrash) + Send + Sync>,
) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    {
        window.with_webview(move |platform| {
            use webkit2gtk::{WebProcessTerminationReason as Reason, WebViewExt as _};
            platform
                .inner()
                .connect_web_process_terminated(move |_, reason| match reason {
                    // Only the app could ask for this, and it never does for the main window.
                    Reason::TerminatedByApi => {}
                    _ => handler(PageCrash::Ended),
                });
        })
    }
    #[cfg(windows)]
    {
        window.with_webview(move |platform| {
            use vw_platform::webview::process_failed_win::{ProcessFailure, on_process_failed};
            let handler = handler.clone();
            let installed = on_process_failed(
                &platform.controller(),
                Box::new(move |failure| {
                    handler(match failure {
                        ProcessFailure::PageEnded => PageCrash::Ended,
                        ProcessFailure::PageUnresponsive => PageCrash::Unresponsive,
                        ProcessFailure::BrowserEnded => PageCrash::EngineEnded,
                    });
                }),
            );
            if let Err(error) = installed {
                tracing::warn!(%error, "the main window's page crashes cannot be watched");
            }
        })
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (window, handler);
        Ok(())
    }
}
