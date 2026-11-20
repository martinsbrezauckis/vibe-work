//! `WebView2`: the end of a webview's own page process (crashed, killed, or no longer answering), reported to the caller
//! on the native UI thread. Processes `WebView2` replaces by itself (GPU, utility, a frame's) are not reported.
//! Depends on: `WebView2`'s `ProcessFailed` event. Called by: vw-webview (the main window's crash recovery).
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE, ICoreWebView2Controller,
    },
    ProcessFailedEventHandler,
};

/// Which of the webview's processes failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFailure {
    /// The page's process ended: a reload starts a new one.
    PageEnded,
    /// The page's process stopped answering: a reload replaces it.
    PageUnresponsive,
    /// `WebView2` itself ended: this webview cannot be reloaded.
    BrowserEnded,
}

/// The failure a `ProcessFailed` kind means for the webview, if any.
#[must_use]
pub fn failure(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> Option<ProcessFailure> {
    match kind {
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => Some(ProcessFailure::PageEnded),
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
            Some(ProcessFailure::PageUnresponsive)
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => {
            Some(ProcessFailure::BrowserEnded)
        }
        _ => None,
    }
}

/// Calls `handler` on the UI thread whenever one of `controller`'s webview processes fails ([`failure`]).
///
/// # Errors
/// The controller has no webview or the event cannot be registered.
pub fn on_process_failed(
    controller: &ICoreWebView2Controller,
    handler: Box<dyn Fn(ProcessFailure)>,
) -> Result<(), String> {
    // SAFETY: the live controller and the callback belong to WebView2's UI thread; the args are read only there.
    unsafe {
        let core = controller.CoreWebView2().map_err(|e| e.to_string())?;
        let mut token = 0;
        core.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED;
                args.ProcessFailedKind(&raw mut kind)?;
                if let Some(failure) = failure(kind) {
                    handler(failure);
                }
                Ok(())
            })),
            &raw mut token,
        )
        .map_err(|e| e.to_string())
    }
}
