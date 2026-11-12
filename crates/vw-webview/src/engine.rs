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
//!     a failed install refuses the mount;
//!     with the feature `macos-proxy` (required in app builds, minimum macOS 14; `mac_guard::MAC_PROXY`) also
//!     the guard's listener of the mount's website data store (`proxy_url_for_store`: `WKWebsiteDataStore`
//!     `proxyConfigurations` belongs to the store, which several mounts can share), set by wry before the webview
//!     exists. `WebKit` sends host NAMES through it (resolved, checked and connected by the guard: DNS rebinding
//!     closed) and address literals (loopback, this computer's own addresses) around it, which the content rules
//!     cover. The proxy route is checked by the `.invalid` canary only
//!     (a loopback canary cannot reach the proxy here by design).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use tauri::{AppHandle, Webview, Wry};
use url::Url;
use vw_preview::accounts::{Provider, ProviderAccount};
use vw_preview::{
    ConsoleEntry, EngineError, EngineFuture, HistoryStep, MountId, MountSpec, Placement,
    PreviewEngine, PreviewHooks, Storage,
};

use crate::platform;

mod close;
mod cookie_verification;
mod dialogs;
mod download_url;
mod mount;
mod native_input;
mod overlay;
mod setup;
mod shields;
mod site_cookies;
mod url_cookies;
mod windows;

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
    page_windows: Mutex<HashMap<String, tauri::Window<Wry>>>,
    request_logs: Mutex<HashMap<String, crate::request_log::Shared>>,
    hooks: OnceLock<Arc<dyn PreviewHooks>>,
    dialogs: Arc<vw_platform::webview::dialogs::Hub>,
    /// Console drain expression per mount label.
    drains: Mutex<HashMap<String, String>>,
    /// Browser environment installed only in the engine's isolated worlds.
    initializers: Mutex<HashMap<String, String>>,
    /// Native pollers for physical page-menu selections; closed with their mount.
    context_pollers: context_menu::Pollers,
    /// Receives browser menu session ids.
    context_sink: Option<context_menu::Sink>,
    /// Visibility per mount label.
    shown: Mutex<HashMap<String, Shown>>,
    /// The mount's existing authenticated egress route, also used by explicit URL downloads.
    download_proxies: Mutex<HashMap<String, String>>,
    /// The person's own pointer activity per mount label, watched on the native view since mount.
    persons: Mutex<HashMap<String, Arc<vw_preview::PersonPointer>>>,
    /// Held by `place` and by a hidden preview's capture (it shows the webview for a moment).
    visibility: Arc<tokio::sync::Mutex<()>>,
    /// Lazy blank webview that owns the shared native cookie manager before any page is mounted.
    account_anchor: tokio::sync::Mutex<Option<Webview<Wry>>>,
    /// Orders native account reads and sign-outs so the UI never reports a stale read across its own deletion.
    account_operations: tokio::sync::Mutex<()>,
    audio_lock: Arc<tokio::sync::Mutex<()>>,
    /// Windows: a session's `WebView2` profile folder (by storage key hex) when it lies outside the session's own
    /// folder (an overridden user data folder), for the wipe.
    foreign_profiles: Mutex<HashMap<String, PathBuf>>,
    /// The agent browser's network guard: the preview instance only.
    guard: Option<vw_netguard::NetGuard>,
    /// This instance requires the build's protection layers: content rules on default macOS, proxy proof elsewhere.
    needs_guard: bool,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    filter_generations: Arc<vw_netguard::rules::RuleGenerations>,
    /// Windows: each native request handler reads a shared, atomically replaced matcher.
    #[cfg(windows)]
    filters: Mutex<HashMap<String, shields::FilterSlot>>,
    /// Windows: tabs reuse the compiled index for the confirmed JSON.
    #[cfg(windows)]
    filter_cache: shields::FilterCache,
    /// Windows: a preview with proxy arguments was refused because its folder already has another process.
    #[cfg(windows)]
    proxy_args_refused: std::sync::atomic::AtomicBool,
    /// macOS: orders rule-list compiles per webview.
    #[cfg(target_os = "macos")]
    rule_generations: Arc<vw_netguard::rules::RuleGenerations>,
}

mod accounts_engine;
mod audio;
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

impl PreviewEngine for TauriEngine {
    fn page_window(
        &self,
        page: &str,
        action: vw_preview::PageWindowAction,
    ) -> EngineFuture<'_, Option<vw_preview::PageWindowInfo>> {
        Box::pin(self.page_window_inner(page.into(), action))
    }
    fn dialog_ownership(&self, id: &MountId, owned: bool) -> Result<(), EngineError> {
        self.update_dialog_ownership(id, owned)
    }

    fn pending_dialog(&self, id: &MountId) -> Option<serde_json::Value> {
        self.dialogs.current(&id.label)
    }
    fn answer_dialog(
        &self,
        id: &MountId,
        dialog: String,
        answer: vw_preview::DialogAnswer,
    ) -> EngineFuture<'_, ()> {
        let id = id.clone();
        Box::pin(async move { self.answer_dialog_inner(&id, dialog, answer).await })
    }

    fn download_url(
        &self,
        id: &MountId,
        url: Url,
        destination: PathBuf,
        max_bytes: u64,
    ) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        let proxy = self
            .download_proxies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id.label)
            .cloned();
        Box::pin(async move {
            if self.needs_guard && proxy.is_none() {
                return Err(EngineError::Platform(
                    "The page's download network route is unavailable.".into(),
                ));
            }
            download_url::run(webview?, url, destination, max_bytes, proxy).await
        })
    }
    fn color_scheme(&self, id: &MountId, dark: Option<bool>) -> EngineFuture<'_, ()> {
        let webview = self.webview(id);
        Box::pin(async move { crate::color_scheme::set(&webview?, dark).await })
    }
    fn network_records(&self, id: &MountId) -> EngineFuture<'_, serde_json::Value> {
        let result = self.webview(id).and_then(|_| {
            let log = self
                .request_logs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id.label)
                .cloned()
                .ok_or(EngineError::NotMounted)?;
            log.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .snapshot()
        });
        Box::pin(std::future::ready(result))
    }
    fn site_cookies(
        &self,
        id: &MountId,
        url: Url,
        request: vw_preview::CookieRequest,
    ) -> EngineFuture<'_, serde_json::Value> {
        let webview = self.webview(id);
        let hooks = self.hooks();
        let label = id.label.clone();
        Box::pin(async move { site_cookies::run(webview?, hooks?, label, url, request).await })
    }
    fn tab_audio(&self, id: &MountId, muted: Option<bool>) -> EngineFuture<'_, serde_json::Value> {
        let webview = self.webview(id);
        Box::pin(async move {
            let _held = self.audio_lock.lock().await;
            crate::platform_audio::tab_audio(&webview?, muted).await
        })
    }
    fn quiet_audio(&self) -> EngineFuture<'_, vw_preview::AudioQuiet> {
        Box::pin(self.quiet_audio_inner())
    }

    fn evaluate_frames(
        &self,
        id: &MountId,
        frame: Option<String>,
        script: String,
    ) -> EngineFuture<'_, Option<Vec<vw_preview::FrameAnswer>>> {
        let webview = self.webview(id);
        let script = self.isolated_script(id, &script);
        Box::pin(async move {
            crate::agent_frames::evaluate(&webview?, frame, script)
                .await
                .map(Some)
        })
    }
    fn evaluate_frames_as_input(
        &self,
        id: &MountId,
        frame: Option<String>,
        script: String,
    ) -> EngineFuture<'_, Option<Vec<vw_preview::FrameAnswer>>> {
        let webview = self.webview(id);
        let script = self.isolated_script(id, &script);
        let label = id.label.clone();
        Box::pin(async move {
            let webview = webview?;
            let input = crate::agent_frames::evaluate_as_input(&webview, frame, script);
            crate::act_window::during(&webview, &label, self.hooks()?, input)
                .await
                .map(Some)
        })
    }
    fn isolated_frame_input(&self) -> bool {
        // Windows: private worlds through the DevTools protocol; a runtime without it answers without a
        // generation, which the checked click refuses.
        cfg!(any(target_os = "linux", target_os = "macos", windows))
    }
    fn supports_content_filter(&self) -> bool {
        true
    }
    fn update_content_filter(&self, id: &MountId, rules: String) -> EngineFuture<'_, ()> {
        let id = id.clone();
        Box::pin(async move {
            self.apply_filter(&self.webview(&id)?, &id.label, rules)
                .await
        })
    }

    fn show_input_marker(
        &self,
        id: &MountId,
        marker: vw_preview::action_tracking::ActionMarker,
    ) -> EngineFuture<'_, ()> {
        let id = id.clone();
        Box::pin(async move {
            let _visibility = self.visibility.lock().await;
            if self
                .shown
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id.label)
                .is_some_and(|shown| shown.visible && !shown.overlay_hidden)
            {
                crate::input_marker::show(&self.webview(&id)?, marker).await?;
            }
            Ok(())
        })
    }

    fn native_click(
        &self,
        id: &MountId,
        point: vw_preview::NativeClickPoint,
        guard: vw_preview::NativeClickGuard,
    ) -> EngineFuture<'_, ()> {
        let id = id.clone();
        Box::pin(async move {
            // A native click gives the page a real user activation: the same act window as a gesture evaluation.
            let webview = self.webview(&id)?;
            let label = id.label.clone();
            let input = self.native_click_inner(id, point, guard);
            crate::act_window::during(&webview, &label, self.hooks()?, input).await
        })
    }

    fn attach(&self, hooks: Arc<dyn PreviewHooks>) {
        let _ = self.hooks.set(hooks);
    }

    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()> {
        let label = spec.id.label.clone();
        Box::pin(async move {
            let _audio = self.audio_lock.try_lock().map_err(|_| {
                EngineError::Platform("Browser audio is paused for a voice answer".into())
            })?;
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
                HistoryStep::ReloadBypassCache => {
                    return crate::reload::bypass_cache(&webview).await;
                }
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
            if self.page_window_native(&page).is_some() {
                return Ok(());
            }
            let overlay_hidden = self
                .shown
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&label)
                .is_some_and(|shown| shown.overlay_hidden);
            if overlay_hidden {
                // A macOS child stays parked outside the window so covering it keeps its selection.
                crate::overlay_visibility::cover(cfg!(target_os = "macos"), || webview.hide())
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
        let own_window = self.page_window_native(&id.session_id).is_some();
        let webview = self.webview(id);
        let label = id.label.clone();
        Box::pin(async move {
            if own_window {
                return Ok(());
            }
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

async fn page_origin_for_mount(
    window: &tauri::Window<Wry>,
    bounds: vw_preview::Rect,
) -> Result<vw_preview::Rect, EngineError> {
    Ok(crate::page_origin::in_window(
        bounds,
        crate::page_origin::css_origin(window).await?,
    ))
}
