//! [`TauriEngine`]: the preview engine on Tauri's multiwebview API. Each mount is a child webview of the main
//! window (`Window::add_child`, feature `unstable`) with the session's own website data, the console script, and
//! every navigation, new-window request, permission request and download routed through the hooks.
//! Mount sequence after ShellX `shellx_browser_engine_runtime.rs:540-740` @ e38c2b5c: create at `about:blank`,
//! place or park, then navigate; close it again when anything after the creation fails. A mount that is not shown
//! is parked hidden at a real viewport size (`MountSpec::parked`), created outside the window on Windows
//! and macOS, so a background session's page lays out like the one the user would see and never covers the UI.
//!
//! Depends on: tauri, vw-preview (engine trait, hooks, storage names), `platform`, `storage`. Called by: the app
//! (built in `startup.rs`, handed to the core as `CoreConfig::preview`).
//! Invariants:
//! - A preview webview gets no capability (the app's capability file grants the bridge to the `main` webview
//!   label only), its label always starts with `vw_preview::LABEL_PREFIX`, and it can never open a window.
//! - Webviews are created and closed from async code on Tokio threads, never from a synchronous command or event
//!   handler (`WebView2` deadlock, Tauri docs for `add_child`).
//! - Permission requests (camera, microphone, geolocation, notifications,...) are always denied.
//! - A physical page-menu click is polled through native evaluation; only the session id is emitted to the main
//!   webview. The selected value stays in the page until the core consumes its one-use choice.
//! - Downloads are written into the app's private staging folder (the hooks decide the path), never straight into a
//!   session folder; the web inspector exists in development builds only.
//! - Windows: a mount is shown only after `platform::secure` saw the session's own `WebView2` profile and
//!   installed the document-request barrier; `place` and the capture of a hidden preview never interleave (the
//!   visibility lock).
//! - Public pages open on every platform under the navigation policy; extra layers per platform,
//!   each reported to the hooks (`network_guard`), none ever blocking a page:
//!   - Linux: every mount sends its traffic through a guard listener of its own (keyed by the label, so loopback
//!     rights belong to the mount), proven by two announced canaries before the first real navigation
//!     (`network_guard(Done(..))`).
//!   - Windows: the request barrier decides every request per webview (`request_check`, layer `RequestBarrier`), and
//!     every preview's traffic goes through the guard's app-wide listener (`proxy_url_all`, browser arguments
//!     `vw_netguard::chromium`, the same for every preview), proven by the same canaries. Where a machine setting
//!     puts every environment into one user data folder (`shared_user_data_folder`), or the arguments are refused,
//!     previews are created with the default arguments and the barrier stays (reported, logged).
//!   - macOS: URL content rules per mount (`rules_macos`, layer `ContentRules`), applied before the first page.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionResponse};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl, Wry,
};
use url::Url;
#[cfg(windows)]
use vw_preview::RequestCheck;
use vw_preview::accounts::{Provider, ProviderAccount};
use vw_preview::{
    ConsoleEntry, EngineError, EngineFuture, GuardCheck, HistoryStep, MountId, MountSpec,
    PageEvent, Placement, PreviewEngine, PreviewHooks, Storage, parse_drain,
};

use crate::platform;

mod native_input;
mod overlay;

/// How long a native evaluation (console drain, pick poll) may take.
const DRAIN_WAIT: Duration = Duration::from_secs(4);

/// What the engine remembers of one mount's visibility (Windows captures hidden previews differently).
#[derive(Clone, Copy, Debug, Default)]
struct Shown {
    /// Shown now (`place` with a placement).
    visible: bool,
    /// The page is temporarily parked behind Browser chrome.
    overlay_hidden: bool,
    /// Last on-screen placement, retained while a chrome overlay parks the child.
    placement: Option<Placement>,
    /// The current size, shown or parked (logical pixels; read on Windows only, for the hidden capture).
    #[cfg_attr(not(windows), allow(dead_code))]
    size: Option<(f64, f64)>,
}

/// Size used to capture a hidden preview whose size is unknown (Windows; every mount records one).
#[cfg(windows)]
const UNSHOWN_SIZE: (f64, f64) = (1280.0, 800.0);

/// The Tauri preview engine for the window `window_label` (the app's `main`).
pub struct TauriEngine {
    app: AppHandle<Wry>,
    window_label: String,
    hooks: OnceLock<Arc<dyn PreviewHooks>>,
    /// Console drain expression per mount label.
    drains: Mutex<HashMap<String, String>>,
    /// Native pollers for physical page-menu selections; closed with their mount.
    context_pollers: context_menu::Pollers,
    /// Receives browser menu session ids.
    context_sink: Option<context_menu::Sink>,
    /// Visibility per mount label.
    shown: Mutex<HashMap<String, Shown>>,
    /// Held by `place` and by a hidden preview's capture (it shows the webview for a moment).
    visibility: tokio::sync::Mutex<()>,
    /// Lazy blank webview that owns the shared native cookie manager before any page is mounted.
    account_anchor: tokio::sync::Mutex<Option<Webview<Wry>>>,
    /// Orders native account reads and sign-outs so the UI never reports a stale read across its own deletion.
    account_operations: tokio::sync::Mutex<()>,
    /// Windows: a session's `WebView2` profile folder (by storage key hex) when it lies outside the session's own
    /// folder (an overridden user data folder), for the wipe.
    foreign_profiles: Mutex<HashMap<String, PathBuf>>,
    /// The agent browser's network guard: the preview instance only.
    guard: Option<vw_netguard::NetGuard>,
    /// Windows: a preview with the guard's browser arguments could not be created once (an environment
    /// with other arguments shares its browser process): later previews use the default arguments.
    #[cfg(windows)]
    proxy_args_refused: std::sync::atomic::AtomicBool,
    /// macOS: orders rule-list compiles per webview.
    #[cfg(target_os = "macos")]
    rule_generations: Arc<vw_netguard::rules::RuleGenerations>,
}

/// How long a mount waits for the guard to see its canary.
const CANARY_WAIT: Duration = Duration::from_secs(4);

mod accounts_engine;
mod context_menu;
mod title;

impl TauriEngine {
    /// An engine placing previews in `window_label`.
    #[must_use]
    pub fn new(app: AppHandle<Wry>, window_label: &str) -> Self {
        Self {
            app,
            window_label: window_label.to_owned(),
            hooks: OnceLock::new(),
            drains: Mutex::new(HashMap::new()),
            context_pollers: context_menu::Pollers::default(),
            context_sink: None,
            shown: Mutex::new(HashMap::new()),
            visibility: tokio::sync::Mutex::new(()),
            account_anchor: tokio::sync::Mutex::new(None),
            account_operations: tokio::sync::Mutex::new(()),
            foreign_profiles: Mutex::new(HashMap::new()),
            guard: None,
            #[cfg(windows)]
            proxy_args_refused: std::sync::atomic::AtomicBool::new(false),
            #[cfg(target_os = "macos")]
            rule_generations: Arc::default(),
        }
    }

    /// The same engine with the agent browser's network guard: every mount uses its proxy.
    #[must_use]
    pub fn with_network_guard(mut self, guard: vw_netguard::NetGuard) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Sends a physical browser context-menu intent to the host as a session id only.
    #[must_use]
    pub fn with_context_sink(mut self, sink: Arc<dyn Fn(String) + Send + Sync>) -> Self {
        self.context_sink = Some(sink);
        self
    }

    /// The proxy URL a new mount gets (`None`: no egress layer for it; public pages open either way).
    /// - Linux: a guard listener of its own (keyed by the label: loopback rights belong to the mount); `WebKitGTK`
    ///   sends every request, loopback included, through the proxy.
    /// - Windows: the guard's app-wide listener, the same for every preview, because every environment
    ///   that shares a `WebView2` browser process must have identical browser arguments; the per-webview request
    ///   barrier keeps each mount to its own rights. None where a machine setting shares one user data folder for
    ///   every environment (the main window's default arguments then decide) or after the arguments were refused.
    /// - macOS: none; the content rules are its extra layer.
    fn proxy_for(&self, id: &MountId) -> Result<Option<String>, EngineError> {
        let Some(guard) = &self.guard else {
            return Ok(None);
        };
        if cfg!(target_os = "linux") {
            return guard
                .proxy_url_for(&id.label)
                .map(Some)
                .map_err(|e| EngineError::Platform(format!("the network guard: {e}")));
        }
        if !(cfg!(windows) && self.proxy_args_usable()) {
            return Ok(None);
        }
        // Windows: a listener that cannot start leaves the preview with the request barrier (as before).
        Ok(guard
            .proxy_url_all()
            .inspect_err(|error| {
                tracing::warn!(%error, "the browser's network guard listener did not start (request barrier)");
            })
            .ok())
    }

    /// Windows: whether previews may get the guard's browser arguments (no shared user data folder, never refused).
    #[cfg(windows)]
    fn proxy_args_usable(&self) -> bool {
        if self
            .proxy_args_refused
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return false;
        }
        if vw_platform::webview::shared_user_data_folder() {
            tracing::info!(
                "previews share one WebView2 user data folder: default browser arguments, request barrier"
            );
            return false;
        }
        true
    }

    #[cfg(not(windows))]
    #[allow(clippy::unused_self)]
    fn proxy_args_usable(&self) -> bool {
        false
    }

    /// Proves that the fresh mount's traffic passes the guard (Linux): a request for a
    /// public-looking name (`*.vw-guard.invalid`, only the proxy can answer it) and a navigation to the listener's
    /// own port on loopback, which reaches the guard in proxy form only when the engine does not bypass the proxy
    /// for loopback. The result is reported (`network_guard(Done(..))`) and shown as the extra layer's state; it
    /// never blocks a page.
    async fn check_guard(
        &self,
        webview: &Webview<Wry>,
        id: &MountId,
        proxy: Option<&str>,
        hooks: &Arc<dyn PreviewHooks>,
    ) {
        let (Some(guard), Some(proxy)) = (&self.guard, proxy) else {
            return;
        };
        // Each canary is announced to the hooks first (the only canary URL the mount may load), then loaded.
        let load = |text: &str| match Url::parse(text) {
            Ok(url) => {
                hooks.network_guard(&id.label, GuardCheck::Expect(url.clone()));
                webview.navigate(url).is_ok()
            }
            Err(_) => false,
        };
        let token = vw_netguard::NetGuard::new_token();
        let web = load(&vw_netguard::NetGuard::canary_url(&token))
            && guard.wait_seen(&token, CANARY_WAIT).await;
        let token = vw_netguard::NetGuard::new_token();
        let port = Url::parse(proxy).ok().and_then(|u| u.port()).unwrap_or(0);
        let loopback = web
            && load(&vw_netguard::NetGuard::loopback_canary_url(proxy, &token))
            && guard.wait_seen_loopback(&token, port, CANARY_WAIT).await;
        if !loopback {
            tracing::warn!(
                web,
                "the browser's network guard is not proven for this webview (navigation policy only)"
            );
        }
        hooks.network_guard(&id.label, GuardCheck::Done(web && loopback));
    }

    /// Records where `label` is now (shown with `placement`, or hidden).
    fn note_shown(&self, label: &str, placement: Option<Placement>) {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = shown.entry(label.to_owned()).or_default();
        entry.visible = placement.is_some();
        if let Some(p) = placement {
            entry.size = Some((p.bounds.width, p.bounds.height));
            entry.placement = Some(p);
        }
    }

    /// Records that `label` is hidden at `placement`'s size (parked).
    fn note_parked(&self, label: &str, placement: Placement) {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = shown.entry(label.to_owned()).or_default();
        entry.visible = false;
        entry.size = Some((placement.bounds.width, placement.bounds.height));
        entry.placement = Some(placement);
    }

    /// Windows: checks the fresh webview's storage and installs the request barrier (`platform::secure`).
    #[cfg(windows)]
    async fn secure(
        &self,
        webview: &Webview<Wry>,
        spec: &MountSpec,
        hooks: &Arc<dyn PreviewHooks>,
    ) -> Result<(), EngineError> {
        let label = spec.id.label.clone();
        let hooks = hooks.clone();
        // every request of the page (documents, scripts, images, fetch) under the mount's rules; a host name
        // whose lookup is still running is held with a deferral, never decided by waiting on the UI thread.
        let decide: vw_platform::webview::DecideRequest = Box::new(move |uri, document| {
            let Ok(url) = Url::parse(uri) else {
                return vw_platform::webview::Verdict::Deny;
            };
            match hooks.request_check(&label, &url, document) {
                RequestCheck::Allow => vw_platform::webview::Verdict::Allow,
                RequestCheck::Deny => {
                    // The scheme and port only: a URL can carry a token.
                    tracing::info!(
                        scheme = url.scheme(),
                        port = ?url.port_or_known_default(),
                        document,
                        "preview request refused before it was sent"
                    );
                    vw_platform::webview::Verdict::Deny
                }
                RequestCheck::Later(register) => vw_platform::webview::Verdict::Later(register),
            }
        });
        let app = self.app.clone();
        let post: vw_platform::webview::PostToUi = Arc::new(move |work| {
            // Fails only when the event loop is gone (quitting): the deferral is never completed, so the held
            // request is never sent (fail closed) and goes away with the webview.
            if let Err(error) = app.run_on_main_thread(work) {
                tracing::warn!(%error, "a held preview request could not be answered (closing?): not sent");
            }
        });
        let facts = platform::secure(webview, spec.storage.key.hex(), decide, post).await?;
        // The barrier checks host names at request time (held with a deferral): the navigation hook never waits.
        self.hooks()?.names_checked_later(&spec.id.label);
        self.hooks()?.network_guard(
            &spec.id.label,
            GuardCheck::Layer(vw_preview::GuardLayer::RequestBarrier),
        );
        tracing::info!(
            user_data_folder = %facts.user_data_folder,
            profile = %facts.profile_name,
            "preview storage (WebView2)"
        );
        let profile = PathBuf::from(&facts.profile_path);
        if !profile.starts_with(&spec.storage.dir) {
            tracing::warn!(
                "the WebView2 user data folder is overridden: the preview runs in its own profile there"
            );
            self.foreign_profiles
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(spec.storage.key.hex(), profile);
        }
        Ok(())
    }

    fn webview(&self, id: &MountId) -> Result<Webview<Wry>, EngineError> {
        self.app
            .get_webview(&id.label)
            .ok_or(EngineError::NotMounted)
    }

    fn hooks(&self) -> Result<Arc<dyn PreviewHooks>, EngineError> {
        self.hooks
            .get()
            .cloned()
            .ok_or_else(|| EngineError::Platform("the preview engine has no hooks yet".to_owned()))
    }
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
        let builder = WebviewBuilder::new(label, WebviewUrl::External(blank))
            .initialization_script(spec.console_script.clone())
            .disable_drag_drop_handler()
            .zoom_hotkeys_enabled(false)
            .general_autofill_enabled(false)
            .focused(false)
            // Inspector only in development builds.
            .devtools(cfg!(debug_assertions))
            .on_navigation(move |url| nav.0.allow_navigation(&nav.1, url))
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
        Ok(crate::storage::with_storage(builder, &spec.storage))
    }
    /// Creates the child webview of `spec` (with the egress `proxy`, or none) in `window`.
    async fn create(
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

    async fn mount_inner(&self, spec: MountSpec) -> Result<(), EngineError> {
        let hooks = self.hooks()?;
        let mut proxy = self.proxy_for(&spec.id)?;
        let window = self
            .app
            .get_window(&self.window_label)
            .ok_or_else(|| EngineError::Platform("the main window is not open".to_owned()))?;
        let parked = platform::parked_or_smallest(spec.parked);
        // a hidden mount is created at its parked size, outside the window (Windows/macOS: the renderer
        // starts at the viewport the page will lay out at; no frame of it can land on the UI before it is hidden).
        // Linux: Tauri packs it into the window's box whatever is asked, so it keeps the smallest size there (as before
        // no frame of it was seen in the box); `park` then moves it, unmapped, into the preview layer at its size.
        let created = if cfg!(target_os = "linux") {
            platform::parked_or_smallest(None)
        } else {
            parked
        };
        let shape = spec.placement.unwrap_or(created);
        let (width, height) = (shape.bounds.width, shape.bounds.height);
        let (x, y) = if spec.placement.is_some() {
            (shape.bounds.x, shape.bounds.y)
        } else {
            platform::outside((width, height))
        };
        let (position, size) = (LogicalPosition::new(x, y), LogicalSize::new(width, height));
        let webview = match self
            .create(&window, &spec, &hooks, proxy.as_deref(), position, size)
            .await
        {
            Ok(webview) => webview,
            // Windows: an environment with other arguments shares the browser process (a shared user
            // data folder the settings check did not see): create it with the default arguments, keep the barrier,
            // and stop asking for the guard's arguments.
            Err(error) if cfg!(windows) && proxy.is_some() => {
                tracing::warn!(%error, "preview created with the default browser arguments (request barrier)");
                #[cfg(windows)]
                self.proxy_args_refused
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                proxy = None;
                self.create(&window, &spec, &hooks, None, position, size)
                    .await?
            }
            Err(error) => return Err(error),
        };
        let setup = async {
            #[cfg(windows)]
            self.secure(&webview, &spec, &hooks).await?;
            match spec.placement {
                Some(placement) => platform::place(&webview, Some(placement)).await?,
                None => platform::park(&webview, parked, false).await?,
            }
            // macOS content rules, in place before the first page (the mount waits for the compile).
            // `VW_DEV_NO_CONTENT_RULES` (development builds only): the falsifier run on the Mac.
            #[cfg(target_os = "macos")]
            if let Some(page) = spec.rules.as_ref().filter(|_| {
                !(cfg!(debug_assertions) && std::env::var_os("VW_DEV_NO_CONTENT_RULES").is_some())
            }) {
                match crate::rules_macos::apply(
                    &webview,
                    &spec.id.label,
                    page,
                    &self.rule_generations,
                )
                .await
                {
                    Ok(()) => hooks.network_guard(
                        &spec.id.label,
                        GuardCheck::Layer(vw_preview::GuardLayer::ContentRules),
                    ),
                    Err(error) => tracing::warn!(%error, "preview content rules not applied"),
                }
            }
            if proxy.is_some() {
                platform::clear_service_workers(&webview, Duration::from_secs(4)).await?;
                // The proxy resolves and checks every host at request time: the navigation hook never waits for a
                // lookup on the UI thread for this mount.
                hooks.names_checked_later(&spec.id.label);
            }
            self.check_guard(&webview, &spec.id, proxy.as_deref(), &hooks)
                .await;
            webview
                .navigate(spec.url.clone())
                .map_err(|e| EngineError::Platform(e.to_string()))
        }
        .await;
        if let Err(error) = setup {
            let _ = webview.close();
            if let Some(guard) = &self.guard {
                guard.forget_session(&spec.id.label);
            }
            return Err(error);
        }
        match spec.placement {
            Some(_) => self.note_shown(&spec.id.label, spec.placement),
            None => self.note_parked(&spec.id.label, parked),
        }
        self.drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(spec.id.label.clone(), spec.console_drain);
        if spec.context_menu
            && let Some(sink) = &self.context_sink
        {
            self.context_pollers.start(webview, &spec.id, sink.clone());
        }
        Ok(())
    }

    async fn drain_inner(&self, id: &MountId) -> Result<Vec<ConsoleEntry>, EngineError> {
        let script = self
            .drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id.label)
            .cloned()
            .ok_or(EngineError::NotMounted)?;
        match self.evaluate_inner(id, script).await {
            Ok(json) => Ok(parse_drain(&json)),
            Err(EngineError::Timeout(_)) => {
                Err(EngineError::Timeout("reading the console".to_owned()))
            }
            Err(other) => Err(other),
        }
    }
}

/// Sends a webview's traffic through the guard's listener `proxy` (`http://127.0.0.1:<port>`). Windows: the whole
/// browser argument string (`vw_netguard::chromium`: wry's defaults, the proxy and `<-loopback>`, which wry's own
/// proxy option cannot express); elsewhere Tauri's proxy option.
fn with_proxy(
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

impl PreviewEngine for TauriEngine {
    fn native_click(
        &self,
        id: &MountId,
        point: vw_preview::NativeClickPoint,
        guard: vw_preview::NativeClickGuard,
    ) -> EngineFuture<'_, ()> {
        Box::pin(self.native_click_inner(id.clone(), point, guard))
    }

    fn attach(&self, hooks: Arc<dyn PreviewHooks>) {
        let _ = self.hooks.set(hooks);
    }

    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()> {
        Box::pin(self.mount_inner(spec))
    }

    fn navigate(&self, id: &MountId, url: Url) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        Box::pin(async move {
            webview?
                .navigate(url)
                .map_err(|e| EngineError::Platform(e.to_string()))
        })
    }

    fn history(&self, id: &MountId, step: HistoryStep) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        Box::pin(async move {
            let webview = webview?;
            let result = match step {
                HistoryStep::Reload => webview.reload(),
                // No native history API in Tauri 2.12: the page's own history object (a page that replaces it
                // only breaks its own back button).
                HistoryStep::Back => webview.eval("history.back()"),
                HistoryStep::Forward => webview.eval("history.forward()"),
            };
            result.map_err(|e| EngineError::Platform(e.to_string()))
        })
    }

    fn place(&self, id: &MountId, placement: Option<Placement>) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        let label = id.label.clone();
        let page = id.session_id.clone();
        Box::pin(async move {
            let _visibility = self.visibility.lock().await;
            let webview = webview?;
            let overlay_hidden = self
                .shown
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&label)
                .is_some_and(|shown| shown.overlay_hidden);
            if overlay_hidden {
                // Keep the child unmapped until the selected page is restored.
                webview
                    .hide()
                    .map_err(|e| EngineError::Platform(e.to_string()))?;
                self.note_shown(&label, None);
            } else {
                platform::place(&webview, placement).await?;
                self.note_shown(&label, placement);
                if std::env::var_os("VW_NATIVE_LAYOUT_TRACE").is_some()
                    && let Some(p) = placement
                {
                    tracing::info!(
                        page = %page,
                        label = %label,
                        x = p.bounds.x,
                        y = p.bounds.y,
                        width = p.bounds.width,
                        height = p.bounds.height,
                        "browser child placed at current layout"
                    );
                }
                #[cfg(target_os = "linux")]
                if placement.is_some() {
                    let _ = platform::trace_browser_overlay(&webview, false, true).await;
                }
            }
            Ok(())
        })
    }

    fn browser_overlay(&self, id: &MountId, hidden: bool) -> EngineFuture<'_, ()> {
        Box::pin(self.set_browser_overlay(id.clone(), hidden))
    }

    fn park(&self, id: &MountId, placement: Placement) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        let label = id.label.clone();
        Box::pin(async move {
            let _visibility = self.visibility.lock().await;
            // Shown meanwhile (a `place` won the lock first): the shown size wins; never hide a shown webview here.
            let visible = self
                .shown
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&label)
                .is_some_and(|s| s.visible);
            if visible {
                return Ok(());
            }
            platform::park(&webview?, placement, true).await?;
            self.note_parked(&label, placement);
            Ok(())
        })
    }

    fn update_rules(
        &self,
        id: &MountId,
        rules: vw_netguard::rules::RulePage,
    ) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        let label = id.label.clone();
        Box::pin(async move {
            #[cfg(target_os = "macos")]
            {
                crate::rules_macos::apply(&webview?, &label, &rules, &self.rule_generations).await
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (webview, label, rules);
                Ok(())
            }
        })
    }

    fn close(&self, id: &MountId) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        // The mount's guard listener goes with it.
        if let Some(guard) = &self.guard {
            guard.forget_session(&id.label);
        }
        self.drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id.label);
        self.context_pollers.stop(&id.label);
        self.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id.label);
        #[cfg(target_os = "macos")]
        let (app, label, generations) = (
            self.app.clone(),
            id.label.clone(),
            self.rule_generations.clone(),
        );
        Box::pin(async move {
            let closed = match webview {
                Ok(webview) => webview
                    .close()
                    .map_err(|e| EngineError::Platform(e.to_string())),
                Err(EngineError::NotMounted) => Ok(()),
                Err(other) => Err(other),
            };
            #[cfg(target_os = "macos")]
            crate::rules_macos::forget(&app, &label, &generations);
            closed
        })
    }

    fn screenshot(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(self.shot(id.clone(), None, Duration::from_secs(7)))
    }

    fn screenshot_full(&self, id: &MountId, size: (u32, u32)) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(self.shot(id.clone(), Some(size), Duration::from_secs(20)))
    }

    fn drain_console(&self, id: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        let id = id.clone();
        Box::pin(async move { self.drain_inner(&id).await })
    }

    fn wipe(&self, storage: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        let storage = storage.clone();
        let foreign = self
            .foreign_profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&storage.key.hex());
        Box::pin(async move { Ok(crate::storage::wipe(&self.app, &storage, foreign).await) })
    }

    fn accounts(&self, storage: &Storage) -> EngineFuture<'_, Vec<ProviderAccount>> {
        let storage = storage.clone();
        Box::pin(async move {
            let _operation = self.account_operations.lock().await;
            let anchor = self.account_anchor(&storage).await?;
            platform::provider_accounts(&anchor).await
        })
    }

    fn sign_out(&self, storage: &Storage, provider: Provider) -> EngineFuture<'_, ()> {
        let storage = storage.clone();
        Box::pin(async move {
            let _operation = self.account_operations.lock().await;
            let anchor = self.account_anchor(&storage).await?;
            platform::sign_out_provider(&anchor, provider).await
        })
    }

    fn evaluate(&self, id: &MountId, script: String) -> EngineFuture<'_, String> {
        let id = id.clone();
        Box::pin(async move { self.evaluate_inner(&id, script).await })
    }
}
