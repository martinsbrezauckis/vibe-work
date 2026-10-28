//! Child webview construction with storage, guarded proxy settings and hook callbacks.
//! Depends on Tauri and preview hooks; called by the engine mount path off the UI thread.

use std::sync::Arc;

use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionResponse};
use tauri::{LogicalPosition, LogicalSize, Webview, WebviewBuilder, WebviewUrl, Wry};
use url::Url;
use vw_preview::{EngineError, MountSpec, PageEvent, PreviewHooks};

use super::{TauriEngine, title};

impl TauriEngine {
    /// The builder of one mount: storage, console script, and every callback routed to `hooks`.
    fn builder(
        spec: &MountSpec,
        hooks: &Arc<dyn PreviewHooks>,
        proxy: Option<&str>,
    ) -> Result<WebviewBuilder<Wry>, EngineError> {
        let blank = Url::parse("about:blank").map_err(|e| EngineError::Platform(e.to_string()))?;
        let label = spec.id.label.clone();
        let (nav, popup, load, title, download) = (
            (hooks.clone(), label.clone()),
            (hooks.clone(), label.clone()),
            (hooks.clone(), label.clone()),
            (hooks.clone(), label.clone()),
            (hooks.clone(), label.clone()),
        );
        let mut builder = WebviewBuilder::new(label, WebviewUrl::External(blank));
        // The page viewer and checker get WebRTC removed in EVERY frame before page scripts run (WebView2:
        // AddScriptToExecuteOnDocumentCreated, which reaches child frames; WKWebView: a user script with
        // forMainFrameOnly NO; WebKitGTK: all frames, though WebRTC is off there anyway).
        if spec.no_peer_connections {
            builder = builder
                .initialization_script_for_all_frames(vw_preview::NO_PEER_CONNECTIONS_SCRIPT);
        }
        let builder = builder
            .initialization_script(spec.console_script.clone())
            .disable_drag_drop_handler()
            .zoom_hotkeys_enabled(false)
            .general_autofill_enabled(false)
            .focused(false)
            // Inspector only in development builds.
            .devtools(cfg!(debug_assertions))
            .on_navigation(move |url| {
                #[cfg(target_os = "macos")]
                if !wry::navigation_context::is_main_frame() {
                    return nav.0.allow_frame_navigation(&nav.1, url);
                }
                nav.0.allow_navigation(&nav.1, url)
            })
            .on_new_window(move |url, _features| {
                popup.0.new_window_refused(&popup.1, &url);
                NewWindowResponse::Deny
            })
            .on_permission_request(|_webview, _kind| PermissionResponse::Deny)
            .on_page_load(move |webview, payload| {
                let url = payload.url().clone();
                let event = match payload.event() {
                    PageLoadEvent::Started => PageEvent::Started(url.clone()),
                    PageLoadEvent::Finished => PageEvent::Probe(url.clone()),
                };
                // Unload a page outside the allowed ports, including a redirect the engine did not check.
                if !load.0.page_event(&load.1, event)
                    && let Ok(blank) = Url::parse("about:blank")
                {
                    let _ = webview.navigate(blank);
                } else if matches!(payload.event(), PageLoadEvent::Finished) {
                    title::finish(&webview, &load.0, &load.1, url);
                }
            })
            .on_document_title_changed(move |webview, _text| {
                title::report(&webview, &title.0, &title.1);
            })
            .on_download(move |_webview, event| match event {
                DownloadEvent::Requested { url, destination } => {
                    let suggested = destination.clone();
                    match download
                        .0
                        .download_destination(&download.1, &url, &suggested)
                    {
                        Some(path) => {
                            *destination = path;
                            true
                        }
                        None => false,
                    }
                }
                DownloadEvent::Finished { url, path, success } => {
                    download
                        .0
                        .download_finished(&download.1, &url, path.as_deref(), success);
                    true
                }
                _ => false,
            });
        let builder = match proxy {
            Some(proxy) => with_proxy(builder, proxy)?,
            None => builder,
        };
        let builder = crate::storage::with_storage(builder, &spec.storage);
        // macOS: a local-network page the user opened gets a data store of its own (`mac_guard::own_store`).
        Ok(if super::mac_guard::own_store(spec) {
            builder.incognito(true)
        } else {
            builder
        })
    }
    /// Creates the child webview of `spec` (with the egress `proxy`, or none) in `window`.
    pub(super) async fn create(
        &self,
        window: &tauri::Window<Wry>,
        spec: &MountSpec,
        hooks: &Arc<dyn PreviewHooks>,
        proxy: Option<&str>,
        position: LogicalPosition<f64>,
        size: LogicalSize<f64>,
    ) -> Result<Webview<Wry>, EngineError> {
        let _ = self;
        let builder = Self::builder(spec, hooks, proxy)?;
        let window = window.clone();
        // `add_child` waits for the UI thread: run it off the async workers.
        tokio::task::spawn_blocking(move || window.add_child(builder, position, size))
            .await
            .map_err(|e| EngineError::Platform(e.to_string()))?
            .map_err(|e| EngineError::Platform(format!("the preview could not be created: {e}")))
    }
}

/// Sends a webview's traffic through the guard's listener `proxy` (`http://127.0.0.1:<port>`). Windows: the whole
/// browser argument string (`vw_netguard::chromium`: wry's defaults, the proxy and `<-loopback>`, which wry's own
/// proxy option cannot express); elsewhere Tauri's proxy option.
pub(super) fn with_proxy(
    builder: WebviewBuilder<Wry>,
    proxy: &str,
) -> Result<WebviewBuilder<Wry>, EngineError> {
    if cfg!(windows) {
        let arguments =
            vw_netguard::chromium::webview2_arguments(proxy).map_err(EngineError::Platform)?;
        return Ok(builder.additional_browser_args(&arguments));
    }
    let url = Url::parse(proxy).map_err(|e| EngineError::Platform(e.to_string()))?;
    Ok(builder.proxy_url(url))
}
