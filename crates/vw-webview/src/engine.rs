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
//! - The visibility lock serializes placement and native capture, including Linux capture restoration.
//! - Windows: a mount is shown only after `platform::secure` checked its own `WebView2` profile and
//!   installed the document-request barrier.
//! - Required previews open only with the protection layers available in this build, before the first page.
//!   Each protection layer is reported to the hooks (`network_guard`):
//!   - Linux: every mount sends its traffic through the guard's listener of its data folder (`folder_guard`:
//!     `WebKitGTK` keeps the proxy per web context, one per folder; the listener grants only what every mount on the
//!     folder may reach), proven by two announced canaries before the first real navigation
//!     (`network_guard(Done(..))`).
//!   - Windows: the request barrier decides every request per webview (`request_check`, layer `RequestBarrier`), and
//!     every preview's traffic goes through the guard's listener of its user data folder (`folder_guard`: browser
//!     arguments `vw_netguard::chromium` with that folder's port; it grants only what every mount on the folder may
//!     reach), proven by the same canaries. Where a setting puts every environment of the app into one user data
//!     folder (`WEBVIEW2_USER_DATA_FOLDER`, the `UserDataFolder` policy; or `WebView2` refuses the arguments because
//!     another environment on the folder has other ones), no preview can have the guard's proxy, so the previews'
//!     instance refuses them with a plain message naming why (`win_guard`: never silently unguarded).
//!   - macOS: URL content rules per mount (`rules_macos`, layer `ContentRules`), applied before the first page;
//!     these alone protect previews in 0.1.2's default build, and a failed install refuses the mount;
//!     with the feature `macos-proxy` (0.1.3, minimum macOS 14; `mac_guard::MAC_PROXY`) also
//!     the guard's listener of the mount's website data store (`proxy_url_for_store`: `WKWebsiteDataStore`
//!     `proxyConfigurations` belongs to the store, which several mounts can share), set by wry before the webview
//!     exists. `WebKit` sends host NAMES through it (resolved, checked and connected by the guard: DNS rebinding
//!     closed) and address literals (loopback, this computer's own addresses) around it, which the content rules
//!     cover; WebTransport, which ignores the proxy, is turned off first. Proven by the `.invalid` canary only
//!     (a loopback canary cannot reach the proxy here by design).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, Wry};
use url::Url;
use vw_preview::accounts::{Provider, ProviderAccount};
use vw_preview::{
    ConsoleEntry, EngineError, EngineFuture, HistoryStep, MountId, MountSpec, Placement,
    PreviewEngine, PreviewHooks, Storage, parse_drain,
};
#[cfg(windows)]
use vw_preview::{GuardCheck, RequestCheck};

use crate::platform;

mod close;
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
    visibility: Arc<tokio::sync::Mutex<()>>,
    /// Lazy blank webview that owns the shared native cookie manager before any page is mounted.
    account_anchor: tokio::sync::Mutex<Option<Webview<Wry>>>,
    /// Orders native account reads and sign-outs so the UI never reports a stale read across its own deletion.
    account_operations: tokio::sync::Mutex<()>,
    /// Windows: a session's `WebView2` profile folder (by storage key hex) when it lies outside the session's own
    /// folder (an overridden user data folder), for the wipe.
    foreign_profiles: Mutex<HashMap<String, PathBuf>>,
    /// The agent browser's network guard: the preview instance only.
    guard: Option<vw_netguard::NetGuard>,
    /// This instance requires the build's protection layers: content rules on default macOS, proxy proof elsewhere.
    needs_guard: bool,
    /// Windows: a preview with the guard's browser arguments could not be created once (an environment
    /// with other arguments shares its browser process): later previews use the default arguments.
    #[cfg(windows)]
    proxy_args_refused: std::sync::atomic::AtomicBool,
    /// macOS: orders rule-list compiles per webview.
    #[cfg(target_os = "macos")]
    rule_generations: Arc<vw_netguard::rules::RuleGenerations>,
}

mod accounts_engine;
mod builder;
mod mac_guard;
#[cfg(target_os = "macos")]
pub(crate) use mac_guard::mac_fault;
use mac_guard::not_checked;
mod context_menu;
mod network_guard;
pub use context_menu::PageMenuChoice;
mod folder_guard;
mod title;
mod win_guard;

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
            visibility: Arc::new(tokio::sync::Mutex::new(())),
            account_anchor: tokio::sync::Mutex::new(None),
            account_operations: tokio::sync::Mutex::new(()),
            foreign_profiles: Mutex::new(HashMap::new()),
            guard: None,
            needs_guard: false,
            #[cfg(windows)]
            proxy_args_refused: std::sync::atomic::AtomicBool::new(false),
            #[cfg(target_os = "macos")]
            rule_generations: Arc::default(),
        }
    }

    /// The same engine with the agent browser's network guard: mounts use its proxy where this build supports it.
    #[must_use]
    pub fn with_network_guard(mut self, guard: vw_netguard::NetGuard) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Requires the build's protection layers before the first page: a guard listener and proven canary on
    /// Linux/Windows and macOS with `macos-proxy`; default macOS (0.1.2) uses content rules only.
    /// Proxied macOS also needs WebTransport off; Windows needs its own guarded data folder.
    /// A missing or unproven layer refuses the mount with a plain error.
    #[must_use]
    pub fn require_network_guard(mut self) -> Self {
        self.needs_guard = true;
        self
    }

    /// Sends a physical browser context-menu click to the host as a session id and the item only.
    #[must_use]
    pub fn with_context_sink(
        mut self,
        sink: Arc<dyn Fn(String, context_menu::PageMenuChoice) + Send + Sync>,
    ) -> Self {
        self.context_sink = Some(sink);
        self
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
    async fn mount_inner(&self, spec: MountSpec) -> Result<(), EngineError> {
        // Windows, Linux: the local-network page the user opened runs on a data folder of its own.
        let spec = folder_guard::with_own_folder(spec);
        let hooks = self.hooks()?;
        let mut proxy = self.proxy_for(&spec)?;
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
        // The UI's rectangles are the app page's CSS pixels; the child is positioned in the content view's.
        let in_window = crate::page_origin::in_window(
            shape.bounds,
            crate::page_origin::css_origin(&window).await?,
        );
        let (x, y) = if spec.placement.is_some() {
            (in_window.x, in_window.y)
        } else {
            platform::outside_at(in_window)
        };
        let (position, size) = (LogicalPosition::new(x, y), LogicalSize::new(width, height));
        let webview = match self
            .create(&window, &spec, &hooks, proxy.as_deref(), position, size)
            .await
        {
            Ok(webview) => webview,
            // Windows: an environment with other arguments already uses the folder (one folder for every window,
            // set in a way the settings check did not see): no preview can have the guard's arguments now. The
            // previews' instance refuses this and every later preview (fail closed); another instance creates it
            // with the default arguments and the barrier.
            Err(error)
                if cfg!(windows) && proxy.is_some() && win_guard::other_arguments(&error) =>
            {
                #[cfg(windows)]
                self.proxy_args_refused
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if self.needs_guard {
                    tracing::warn!(%error, "WebView2 refused the guard's browser arguments: previews are off");
                    return Err(win_guard::previews_off(win_guard::OneFolder::Refused));
                }
                tracing::warn!(%error, "preview created with the default browser arguments (request barrier)");
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
            #[cfg(target_os = "macos")]
            self.macos_layers(&webview, &spec, &hooks, proxy.is_some())
                .await?;
            // macOS: reaching here with a proxy means WebTransport is off (else the mount was refused above).
            let bypass_closed = true;
            // Every guarded web mount (a browser/preview mount: URL rules, or the guard's proxy) starts without the
            // service workers earlier pages registered in its storage, on every platform.
            platform::clear_service_workers_at_mount(
                &webview,
                proxy.is_some() || spec.rules.is_some(),
            )
            .await?;
            if proxy.is_some() {
                // The proxy resolves and checks every host at request time: the navigation hook never waits for a
                // lookup on the UI thread for this mount.
                hooks.names_checked_later(&spec.id.label);
            }
            let proven = self
                .check_guard(&webview, &spec.id, proxy.as_deref(), bypass_closed, &hooks)
                .await;
            if (self.proxy_required() || cfg!(target_os = "macos") && proxy.is_some()) && !proven {
                return Err(not_checked("its network check could not be confirmed"));
            }
            webview
                .navigate(spec.url.clone())
                .map_err(|e| EngineError::Platform(e.to_string()))
        }
        .await;
        if let Err(error) = setup {
            let _ = Self::close_webview(&webview);
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
        let label = spec.id.label.clone();
        Box::pin(async move {
            let mounted = self.mount_inner(spec).await;
            // A failed setup cannot keep authority through its guard listener or store membership.
            if mounted.is_err()
                && let Some(guard) = &self.guard
            {
                guard.forget_session(&label);
            }
            mounted
        })
    }

    fn navigate(&self, id: &MountId, url: Url) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        Box::pin(async move {
            webview?
                .navigate(url)
                .map_err(|e| EngineError::Platform(e.to_string()))
        })
    }

    fn continue_navigation(
        &self,
        id: &MountId,
        url: Url,
        current: Box<dyn FnOnce() -> bool + Send>,
    ) -> Result<(), EngineError> {
        let webview = self.webview(id)?;
        let app = self.app.clone();
        // Always leave the navigation hook first, even if DNS completed during callback registration.
        tauri::async_runtime::spawn(async move {
            if let Err(error) = app.run_on_main_thread(move || {
                if current() {
                    // The pinned tauri-runtime-wry Context::send_user_message dispatches inline on
                    // the UI thread, through Navigate -> load_url. There is no unchecked second hop.
                    if let Err(error) = webview.navigate(url) {
                        tracing::warn!(%error, "held preview navigation could not load");
                    }
                }
            }) {
                tracing::warn!(%error, "held preview navigation could not reach UI (closing?)");
            }
        });
        Ok(())
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
        let id = id.clone();
        Box::pin(async move { self.close_inner(&id) })
    }

    fn screenshot(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(self.shot(id.clone(), None, Duration::from_secs(7), false))
    }

    fn screenshot_slice(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(self.shot(id.clone(), None, Duration::from_secs(7), true))
    }

    fn print_pdf(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(self.pdf(id.clone()))
    }

    fn screenshot_full(&self, id: &MountId, size: (u32, u32)) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(self.shot(id.clone(), Some(size), Duration::from_secs(20), false))
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
        Box::pin(async move {
            let left = crate::storage::wipe(&self.app, &storage, foreign).await;
            // macOS: the store is gone; its guard listener goes with it (a new store with this key gets a new port).
            // Windows: the folder is gone, so its browser process (and the local-network page folders inside it) has
            // ended; their listeners go with them. A folder still in use keeps its listener and port. Linux: the web
            // context outlives the webview until quit and keeps its proxy, so its listener stays.
            if let Some(guard) = &self.guard {
                if cfg!(target_os = "macos") {
                    guard.forget_store(&storage.key.hex());
                } else if cfg!(windows) && !storage.dir.exists() {
                    guard.forget_store(&folder_guard::store_key(&storage.dir));
                    guard.forget_stores_under(&folder_guard::inner_store_prefix(&storage.dir));
                }
            }
            Ok(left)
        })
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

    fn view_size(&self, id: &MountId) -> EngineFuture<'_, Option<(f64, f64)>> {
        let webview = self.webview(id);
        Box::pin(async move {
            let webview = webview?;
            let bounds = webview
                .bounds()
                .map_err(|e| EngineError::Platform(e.to_string()))?;
            let scale = webview
                .window()
                .scale_factor()
                .map_err(|e| EngineError::Platform(e.to_string()))?;
            let size = bounds.size.to_logical::<f64>(scale);
            Ok(Some((size.width, size.height)))
        })
    }
}
