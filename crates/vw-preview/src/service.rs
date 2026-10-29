//! `PreviewService`: the per-session previews the core drives (open, history, width, layout, close, screenshot,
//! console, session end), on top of a [`PreviewEngine`]. Independent page keys share a session owner; the pool
//! suspends eligible least-recently-used background pages, and the URL policy (`policy`) guards each mount.
//!
//! Depends on: `engine`, `state`, `hooks`, `layout`, `shot`, `console`, `policy`, tokio. Called by: vw-core-api
//! (UI and Debug API commands, the agent tools, session end, quit).
//! Invariants:
//! - The registry lock is never held across an engine call (engine calls may wait on the UI thread).
//! - Callers pass URLs already checked against the session (`UrlPolicy::check`); the service checks the shape
//!   again before every mount or navigation.
//! - Every engine call is bounded by [`PreviewConfig::engine_timeout`] (screenshots by `shot_timeout`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use url::Url;

use crate::engine::{EngineError, EngineFuture, HistoryStep, MountId, PreviewEngine, Storage};
use crate::engine_pool::PoolState;
use crate::hooks::{Hooks, Inner};
use crate::layout::{self, DeviceWidth, Rect};
use crate::policy::UrlPolicy;
pub use crate::preview_error::PreviewError;
use crate::shot::{self, Shot};
use crate::state::{BrowserMountState, PreviewView, Shown, State};
use crate::visibility::VisibilityReport;

#[path = "service_accounts.rs"]
mod service_accounts;
#[path = "service_console.rs"]
mod service_console;
#[path = "service_favicon.rs"]
mod service_favicon;
#[path = "service_layout.rs"]
mod service_layout;
#[path = "service_lifecycle.rs"]
mod service_lifecycle;
#[path = "service_mount.rs"]
mod service_mount;
#[path = "service_overlay.rs"]
mod service_overlay;
#[path = "service_pool.rs"]
mod service_pool;
#[path = "service_server.rs"]
mod service_server;
#[path = "service_visibility.rs"]
mod service_visibility;

/// Settings.
#[derive(Clone)]
pub struct PreviewConfig {
    /// Folder holding one data folder per session (`<root>/<key hex>`), user-only.
    pub storage_root: PathBuf,
    /// Previews mounted at once. Default 4.
    pub live_cap: usize,
    /// Memory admission probe. None uses live system and cgroup memory.
    pub memory_probe: Option<crate::MemoryProbe>,
    /// Fixed URL rules (the app's dev origin).
    pub policy: UrlPolicy,
    /// Longest wait for a page load after `open` (the answer still returns, with `loading: true`). Default 8 s.
    pub load_wait: Duration,
    /// Longest engine operation. Default 10 s.
    pub engine_timeout: Duration,
    /// Longest screenshot (brief: < 10 s). Default 8 s.
    pub shot_timeout: Duration,
    /// Downloads above this size are dropped. Default 2 GiB.
    pub max_download_bytes: u64,
    /// How host names are looked up for the navigation hooks and the request barrier (`None`: the system resolver;
    /// development test runs slow some names down, `VW_DEV_SLOW_NAMES`).
    pub name_resolver: Option<crate::names::NameResolver>,
    /// Exact loopback test origin treated as a public browser profile (test feature only).
    #[cfg(feature = "e2e-script")]
    pub test_browser_origin: Option<Url>,
    /// Additional exact test origins that consume external browser slots.
    #[cfg(feature = "e2e-script")]
    pub test_pool_origins: Vec<Url>,
}

impl std::fmt::Debug for PreviewConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("PreviewConfig");
        debug
            .field("storage_root", &self.storage_root)
            .field("live_cap", &self.live_cap)
            .field("memory_probe", &self.memory_probe.is_some())
            .field("policy", &self.policy)
            .field("load_wait", &self.load_wait)
            .field("engine_timeout", &self.engine_timeout)
            .field("shot_timeout", &self.shot_timeout)
            .field("max_download_bytes", &self.max_download_bytes)
            .field("name_resolver", &self.name_resolver.is_some());
        #[cfg(feature = "e2e-script")]
        debug.field("test_browser_origin", &self.test_browser_origin);
        #[cfg(feature = "e2e-script")]
        debug.field("test_pool_origins", &self.test_pool_origins);
        debug.finish()
    }
}

impl PreviewConfig {
    /// Defaults with `storage_root`.
    #[must_use]
    pub fn new(storage_root: PathBuf) -> Self {
        Self {
            storage_root,
            live_cap: 4,
            memory_probe: None,
            policy: UrlPolicy::default(),
            load_wait: Duration::from_secs(8),
            engine_timeout: Duration::from_secs(10),
            shot_timeout: Duration::from_secs(8),
            max_download_bytes: crate::downloads::DEFAULT_MAX_BYTES,
            name_resolver: None,
            #[cfg(feature = "e2e-script")]
            test_browser_origin: None,
            #[cfg(feature = "e2e-script")]
            test_pool_origins: Vec::new(),
        }
    }
}

/// What `open` needs besides the URL.
#[derive(Clone, Debug)]
pub struct OpenRequest {
    /// The session.
    pub session_id: String,
    /// The checked URL.
    pub url: Url,
    /// Ports the preview may navigate to afterwards (the session's attributed + leased ports).
    pub allowed_ports: Vec<u16>,
    /// The session's `downloads/` folder (`None`: downloads are cancelled).
    pub downloads_dir: Option<PathBuf>,
    /// Wait for the load to finish (bounded by `load_wait`).
    pub wait: bool,
    /// The USER opened a page on the local network (a private-network literal, or a name the core found resolving
    /// there): it gets a Lan mount reaching only its own host. Never set for the agent.
    pub lan: bool,
}

/// The previews of all sessions.
#[derive(Clone)]
pub struct PreviewService {
    pub(crate) inner: Arc<Inner>,
    pub(crate) engine: Arc<dyn PreviewEngine>,
    config: Arc<PreviewConfig>,
    pool: Arc<std::sync::Mutex<PoolState>>,
    pool_op: Arc<tokio::sync::Mutex<()>>,
    activity: Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>,
        >,
    >,
    pub(crate) layout_op: Arc<tokio::sync::Mutex<()>>,
    /// Refuses placements of a replaced main document; checked and advanced under `layout_op`.
    pub(crate) placement_gate: Arc<crate::PlacementGate>,
    pub(crate) visibility: Arc<std::sync::Mutex<Option<VisibilityReport>>>,
    pub(crate) visibility_changes: Arc<tokio::sync::watch::Sender<u64>>,
    pub(crate) native_window_visible: Arc<std::sync::Mutex<Option<bool>>>,
}

impl PreviewService {
    /// A change-driven answer for a pending show request; subscribe before emitting the request.
    #[must_use]
    pub fn visibility_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.visibility_changes.subscribe()
    }

    /// First current reason a screenshot cannot show this page, if any.
    #[must_use]
    pub fn screenshot_unavailable_reason(&self, page_key: &str) -> Option<String> {
        match self.require_visible_screenshot(page_key) {
            Err(PreviewError::ScreenshotHidden(reason)) => Some(reason),
            _ => None,
        }
    }

    /// A service over `engine` (its hooks are attached here).
    #[must_use]
    pub fn new(engine: Arc<dyn PreviewEngine>, config: PreviewConfig) -> Self {
        let inner = Arc::new(Inner {
            engine: Arc::downgrade(&engine),
            guard_refused: std::sync::Mutex::new(None),
            state: std::sync::Mutex::new(State::default()),
            policy: config.policy.clone(),
            changes: tokio::sync::broadcast::channel(64).0,
            staging: config.storage_root.join("downloads-staging"),
            max_download_bytes: config.max_download_bytes,
            guard: std::sync::Mutex::new(None),
            names: crate::names::NameCheck::new(),
            rules_lock: tokio::sync::Mutex::new(()),
        });
        if let Some(resolver) = &config.name_resolver {
            inner.names.set_resolver(resolver.clone());
        }
        engine.attach(Arc::new(Hooks(inner.clone())));
        let pool = PoolState::load(&config.storage_root, config.memory_probe.clone());
        Self {
            inner,
            engine,
            config: Arc::new(config),
            pool: Arc::new(std::sync::Mutex::new(pool)),
            pool_op: Arc::new(tokio::sync::Mutex::new(())),
            activity: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            layout_op: Arc::new(tokio::sync::Mutex::new(())),
            placement_gate: Arc::default(),
            visibility: Arc::new(std::sync::Mutex::new(None)),
            visibility_changes: Arc::new(tokio::sync::watch::Sender::new(0)),
            native_window_visible: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Where `host` points, waiting for the lookup on a blocking thread (the core before an open). Shares the cache
    /// the navigation hooks and the request barrier read, so a page the core just checked never waits there.
    pub async fn check_name(&self, host: &str) -> crate::names::NameVerdict {
        let (names, name) = (self.inner.names.clone(), host.to_owned());
        tokio::task::spawn_blocking(move || names.resolve(&name))
            .await
            .unwrap_or_else(|_| {
                crate::names::NameVerdict::Refused(format!(
                    "could not check where {host} points; try again"
                ))
            })
    }

    /// The name check's sizes (tests and diagnostics: the bounds hold under a flood of unique names).
    #[must_use]
    pub fn name_check_stats(&self) -> crate::names::NameStats {
        self.inner.names.stats()
    }

    /// Replaces how in-place navigations look up host names (the system resolver by default; tests script it).
    pub fn set_name_resolver(&self, resolver: crate::names::NameResolver) {
        self.inner.names.set_resolver(resolver);
    }

    /// Session ids whose preview changed (page loads, titles, refusals, downloads, open, close). A lagging
    /// receiver should re-read every preview.
    #[must_use]
    pub fn changes(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.inner.changes.subscribe()
    }

    /// The URL policy (for the core's checks).
    #[must_use]
    pub fn policy(&self) -> &UrlPolicy {
        &self.inner.policy
    }

    fn session_mutex(&self, session_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .activity
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks.retain(|_, weak| weak.strong_count() > 0);
        if let Some(lock) = locks.get(session_id).and_then(std::sync::Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(session_id.to_owned(), Arc::downgrade(&lock));
            lock
        }
    }

    pub(super) async fn session_lock(&self, session_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.session_mutex(session_id).lock_owned().await
    }

    pub(super) fn try_session_lock(
        &self,
        session_id: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.session_mutex(session_id).try_lock_owned().ok()
    }

    /// The storage of a session's preview.
    #[must_use]
    pub fn storage(&self, session_id: &str) -> Storage {
        let owner = self.inner.lock().owner_of(session_id);
        let key = crate::ids::StorageKey::of(&owner);
        Storage {
            key,
            dir: key.data_dir(&self.config.storage_root),
        }
    }

    /// A session's preview, if it ever opened one.
    #[must_use]
    pub fn view(&self, session_id: &str) -> Option<PreviewView> {
        let state = self.inner.lock();
        let shown = state.is_shown(session_id);
        state.entries.get(session_id).map(|e| e.view(shown))
    }

    /// Every session's preview.
    #[must_use]
    pub fn views(&self) -> Vec<PreviewView> {
        let state = self.inner.lock();
        let mut all: Vec<PreviewView> = state
            .entries
            .values()
            .map(|e| e.view(state.is_shown(&e.session_id)))
            .collect();
        all.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        all
    }

    /// Opens `request.url` in the session's preview (mounting it when needed) and, with `wait`, waits for the
    /// load to finish (bounded).
    ///
    /// # Errors
    /// [`PreviewError`] when the URL's shape is refused or the engine fails.
    pub async fn open(&self, request: OpenRequest) -> Result<PreviewView, PreviewError> {
        self.cancel_pending_navigation(&request.session_id);
        let _activity = self.session_lock(&request.session_id).await;
        let destination = self.preflight_page_open(&request)?;
        let session = request.session_id.clone();
        self.close_unready(&session).await?;
        let lan_host = request
            .lan
            .then(|| request.url.host_str().map(str::to_owned))
            .flatten();
        let class = if lan_host.is_some() {
            crate::policy::PageClass::Lan
        } else {
            destination.class()
        };
        // A mount keeps its page class (and a Lan mount its host); a page of another class or Lan
        // host gets a fresh mount (the old page's requests can never use the new page's rights).
        let other_class = {
            let state = self.inner.lock();
            state.entries.get(&session).is_some_and(|e| {
                e.mount.is_some()
                    && (e.mount_class() != class || e.mount_lan_host() != lan_host.as_deref() || {
                        #[cfg(feature = "e2e-script")]
                        {
                            let fixture = self.config.test_browser_origin.as_ref();
                            let old = e.url.as_ref().is_some_and(|url| {
                                fixture.is_some_and(|origin| origin.origin() == url.origin())
                            });
                            let new = fixture
                                .is_some_and(|origin| origin.origin() == request.url.origin());
                            old != new
                        }
                        #[cfg(not(feature = "e2e-script"))]
                        {
                            false
                        }
                    })
            })
        };
        if other_class {
            self.unmount(&session).await?;
        }
        let (mount, loads_before, mut loads, placement) = {
            let mut state = self.inner.lock();
            let now = state.tick();
            let area = state.shown.clone();
            let entry = state.entry(&session);
            entry.cancel_navigation();
            entry.reset_console();
            entry.pending_previous_url = entry.url.replace(request.url.clone());
            entry.title = None;
            entry.pending_open = Some(request.url.clone());
            entry.pending_requested_admitted = None;
            entry.pending_admitted_urls.clear();
            entry.pending_started = false;
            entry.lan_host.clone_from(&lan_host);
            entry.allowed.clone_from(&request.allowed_ports);
            entry.downloads_dir.clone_from(&request.downloads_dir);
            entry.last_used = now;
            entry.loading = true;
            entry.browser_mount = BrowserMountState::Ready;
            let loads = entry.loads.subscribe();
            let before = *loads.borrow();
            let placement = area
                .filter(|s| s.session_id == session)
                .and_then(|s| layout::place(s.area, entry.width).ok());
            (entry.mount.clone(), before, loads, placement)
        };
        // Local mounts' URL rules follow the ports (any session's open may change them), before the page
        // that needs them loads.
        self.refresh_rules().await;
        if let Some(mount) = mount {
            let id = self.mount_id(&session, &mount);
            self.bounded("navigate", self.engine.navigate(&id, request.url.clone()))
                .await?;
        } else {
            self.mount(&session, request.url.clone(), placement).await?;
        }
        if let Some(used) = self.lock_pool().active.get_mut(&session) {
            *used = tokio::time::Instant::now();
        }
        self.enforce_cap(&session).await;
        if request.wait {
            let _ =
                tokio::time::timeout(self.config.load_wait, loads.wait_for(|n| *n > loads_before))
                    .await;
        }
        self.inner.lock().entry(&session).server_status = None;
        self.inner.changed(&session);
        self.view(&session).ok_or(PreviewError::NothingOpen)
    }

    /// Back, forward or reload.
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], or the engine's failure.
    pub async fn history(&self, session_id: &str, step: HistoryStep) -> Result<(), PreviewError> {
        self.cancel_pending_navigation(session_id);
        let _activity = self.session_lock(session_id).await;
        let id = self.ensure_mounted(session_id).await?;
        // Flush old-page messages before scheduling history. A no-target back/forward sends no
        // Started callback; its still-current page may report new diagnostics immediately.
        let _ = self
            .bounded("console", self.engine.drain_console(&id))
            .await;
        {
            let mut state = self.inner.lock();
            let entry = state.entry(session_id);
            entry.cancel_navigation();
            entry.reset_console();
        }
        self.bounded("history", self.engine.history(&id, step))
            .await?;
        Ok(())
    }

    /// Sets the device width: re-places the preview when it is shown, re-parks it at the new width when it is mounted
    /// but hidden.
    ///
    /// # Errors
    /// The engine's failure while re-placing.
    pub async fn set_width(
        &self,
        session_id: &str,
        width: DeviceWidth,
    ) -> Result<(), PreviewError> {
        let _activity = self.session_lock(session_id).await;
        let placement = {
            let mut state = self.inner.lock();
            let shown = state.shown.clone();
            let last_area = state.last_area;
            let entry = state.entry(session_id);
            entry.width = width;
            match (shown, entry.ready_mount()) {
                (Some(s), Some(m)) if s.session_id == session_id => layout::place(s.area, width)
                    .ok()
                    .map(|p| (m.clone(), p, true)),
                // A hidden preview is parked again at its new device width.
                (_, Some(m)) => Some((m.clone(), layout::parked(last_area, width), false)),
                _ => None,
            }
        };
        if let Some((mount, placement, shown)) = placement {
            let id = self.mount_id(session_id, &mount);
            if shown {
                self.bounded("place", self.engine.place(&id, Some(placement)))
                    .await?;
            } else {
                self.bounded("park", self.engine.park(&id, placement))
                    .await?;
            }
        }
        Ok(())
    }

    /// What the window shows: `Some((session, area))` places that session's preview there (mounting it again
    /// when suspended) and hides every other one; `None` hides all.
    ///
    /// # Errors
    /// [`PreviewError::Layout`] for an invalid area; engine failures of the shown preview.
    pub async fn layout(&self, shown: Option<(String, Rect)>) -> Result<(), PreviewError> {
        self.layout_selected(shown, service_layout::PanelSelection::Unchanged)
            .await
    }

    /// Applies a passive UI layout report only while its page is still the selected tab.
    /// A queued report from a tab switched away from cannot restore its native view.
    ///
    /// # Errors
    /// A rectangle without a page, invalid placement, or native engine failure.
    pub async fn report_tab_layout(
        &self,
        page: Option<String>,
        rect: Option<Rect>,
    ) -> Result<(), PreviewError> {
        let _layout = self.layout_op.lock().await;
        if self.stale_window_layout() {
            return Ok(());
        }
        tracing::debug!(
            page = page.as_deref().unwrap_or("-"),
            ?rect,
            "preview layout reported"
        );
        let Some(page_key) = page else {
            if rect.is_some() {
                return Err(PreviewError::InvalidTab(
                    "a Browser rectangle needs its page".to_owned(),
                ));
            }
            return self
                .layout_selected_locked(None, service_layout::PanelSelection::Set(None))
                .await;
        };
        let current = {
            let state = self.inner.lock();
            let owner = state.owner_of(&page_key);
            state
                .tabs
                .get(&owner)
                .and_then(|tabs| tabs.selected.as_deref())
                .unwrap_or(&owner)
                == page_key.as_str()
        };
        if !current {
            tracing::debug!(page = %page_key, "layout ignored: the page is not its session's selected tab");
            return Ok(());
        }
        // A Browser panel reopening can reclaim an engine released while the page was away.
        // Native-only layout changes intentionally keep Released parked.
        if rect.is_some()
            && let Some(entry) = self.inner.lock().entries.get_mut(&page_key)
            && entry.browser_mount == BrowserMountState::Released
            && entry.server_status.is_none()
        {
            entry.browser_mount = BrowserMountState::Ready;
        }
        self.layout_selected_locked(
            rect.map(|area| (page_key.clone(), area)),
            service_layout::PanelSelection::Set(Some(page_key)),
        )
        .await
    }

    async fn layout_selected(
        &self,
        shown: Option<(String, Rect)>,
        selected: service_layout::PanelSelection,
    ) -> Result<(), PreviewError> {
        let _layout = self.layout_op.lock().await;
        if self.stale_window_layout() {
            return Ok(());
        }
        self.layout_selected_locked(shown, selected).await
    }

    /// Caller holds `layout_op`; places a selected page at the still-current Browser area.
    pub(crate) async fn layout_page_locked(
        &self,
        page_key: String,
        area: Rect,
    ) -> Result<(), PreviewError> {
        self.layout_selected_locked(
            Some((page_key.clone(), area)),
            service_layout::PanelSelection::Set(Some(page_key)),
        )
        .await
    }

    /// Caller holds `layout_op`, so visibility checks and native placement share one ordering.
    async fn layout_selected_locked(
        &self,
        shown: Option<(String, Rect)>,
        selected: service_layout::PanelSelection,
    ) -> Result<(), PreviewError> {
        if let Some((_, area)) = &shown {
            area.checked().map_err(PreviewError::Layout)?;
        }
        if let service_layout::PanelSelection::Set(session) = selected {
            self.select_browser_panel(session).await;
        }
        // Hiding (including a modal or session switch) parks the view, never its document.
        let previously_shown = self
            .inner
            .lock()
            .shown
            .as_ref()
            .map(|s| s.session_id.clone());
        tracing::debug!(
            shown = shown.as_ref().map_or("-", |(page, _)| page.as_str()),
            previously = previously_shown.as_deref().unwrap_or("-"),
            "preview layout applied"
        );
        if let Some(session) = previously_shown
            && shown.as_ref().is_none_or(|(next, _)| next != &session)
        {
            self.capture_tab_snapshot(&session).await;
            self.touch_pool(&session);
        }
        let (others, target) = {
            let mut state = self.inner.lock();
            state.shown = shown
                .clone()
                .map(|(session_id, area)| Shown { session_id, area });
            if let Some((_, area)) = &shown {
                state.last_area = Some(*area);
            }
            let now = state.tick();
            if shown.is_none() {
                for entry in state.entries.values_mut() {
                    if entry.browser_mount == BrowserMountState::Released {
                        entry.browser_mount = BrowserMountState::Ready;
                    }
                }
            }
            let target = shown.as_ref().and_then(|(session, _)| {
                state.entries.get_mut(session)?.last_used = now;
                Some(session.clone())
            });
            let others: Vec<MountId> = state
                .entries
                .values()
                .filter(|e| shown.as_ref().is_none_or(|(s, _)| *s != e.session_id))
                .filter_map(|e| {
                    e.ready_mount().map(|m| MountId {
                        session_id: e.session_id.clone(),
                        label: m.label.clone(),
                    })
                })
                .collect();
            (others, target)
        };
        for id in others {
            let _activity = self.session_lock(&id.session_id).await;
            let current = self
                .inner
                .lock()
                .entries
                .get(&id.session_id)
                .and_then(|entry| entry.mount.as_ref())
                .is_some_and(|mount| mount.label == id.label);
            if current {
                let _ = self.bounded("hide", self.engine.place(&id, None)).await;
            }
        }
        if let Some(session) = target {
            self.apply_layout_target(&session).await?;
        }
        Ok(())
    }

    /// A shaped screenshot of the session's visible preview.
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], engine failures and timeouts, or [`PreviewError::Image`].
    pub async fn screenshot(&self, session_id: &str) -> Result<Shot, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        if self.shots_blocked(session_id) {
            return Err(PreviewError::Image(crate::browse::SHOTS_BLOCKED.to_owned()));
        }
        self.require_visible_screenshot(session_id)?;
        let id = self.ensure_mounted(session_id).await?;
        let png = bounded(
            self.config.shot_timeout,
            "screenshot",
            self.engine.screenshot(&id),
        )
        .await?;
        shot::shape(&png).map_err(PreviewError::Image)
    }

    /// The engine's own PNG of what the session's preview shows, at full size for stitching and comparing.
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], engine failures and timeouts.
    pub async fn screenshot_png(&self, session_id: &str) -> Result<Vec<u8>, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        if self.shots_blocked(session_id) {
            return Err(PreviewError::Image(crate::browse::SHOTS_BLOCKED.to_owned()));
        }
        self.require_visible_screenshot(session_id)?;
        let id = self.ensure_mounted(session_id).await?;
        Ok(bounded(
            self.config.shot_timeout,
            "screenshot",
            self.engine.screenshot(&id),
        )
        .await?)
    }

    /// A full-page capture's viewport slice, at the page's real resolution where supported.
    ///
    /// # Errors
    /// Nothing open, a hidden/protected page, engine failures and timeouts.
    pub async fn screenshot_slice_png(&self, session_id: &str) -> Result<Vec<u8>, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        if self.shots_blocked(session_id) {
            return Err(PreviewError::Image(crate::browse::SHOTS_BLOCKED.to_owned()));
        }
        self.require_visible_screenshot(session_id)?;
        let id = self.ensure_mounted(session_id).await?;
        Ok(bounded(
            self.config.shot_timeout,
            "viewport slice",
            self.engine.screenshot_slice(&id),
        )
        .await?)
    }

    /// The engine's PNG of the whole document of CSS size `size` in one render, `None` when the engine has no such
    /// capture (macOS; the caller stitches).
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], engine failures and timeouts.
    pub async fn screenshot_full_png(
        &self,
        session_id: &str,
        size: (u32, u32),
    ) -> Result<Option<Vec<u8>>, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        if self.shots_blocked(session_id) {
            return Err(PreviewError::Image(crate::browse::SHOTS_BLOCKED.to_owned()));
        }
        self.require_visible_screenshot(session_id)?;
        let id = self.ensure_mounted(session_id).await?;
        match bounded(
            self.config.shot_timeout * 3,
            "full-page screenshot",
            self.engine.screenshot_full(&id, size),
        )
        .await
        {
            Ok(png) => Ok(Some(png)),
            Err(EngineError::Unsupported(_)) => Ok(None),
            Err(other) => Err(other.into()),
        }
    }

    /// Evaluates `script` natively in the session's preview (mounted again first when suspended) and answers the
    /// JSON of its value: untrusted page data (the element picker).
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], engine failures and timeouts.
    pub async fn evaluate(&self, session_id: &str, script: String) -> Result<String, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        let id = self.ensure_mounted(session_id).await?;
        Ok(self
            .bounded("evaluate", self.engine.evaluate(&id, script))
            .await?)
    }

    /// Closes a session's preview and forgets its URL (the user closed it).
    ///
    /// # Errors
    /// The engine's failure while closing.
    pub async fn close(&self, session_id: &str) -> Result<(), PreviewError> {
        self.cancel_pending_navigation(session_id);
        let _activity = self.session_lock(session_id).await;
        self.unmount(session_id).await?;
        if let Some(entry) = self.inner.lock().entries.get_mut(session_id) {
            entry.reset_console();
            entry.url = None;
            entry.title = None;
            entry.loading = false;
            entry.server_status = None;
            entry.pending_open = None;
            entry.pending_previous_url = None;
            entry.pending_requested_admitted = None;
            entry.pending_admitted_urls.clear();
            entry.pending_started = false;
            // A closed preview's ports no longer count for the other previews' rules.
            entry.allowed.clear();
        }
        self.refresh_rules().await;
        self.inner.changed(session_id);
        Ok(())
    }
}

async fn bounded<T>(
    limit: Duration,
    what: &str,
    future: EngineFuture<'_, T>,
) -> Result<T, EngineError> {
    tokio::time::timeout(limit, future)
        .await
        .map_err(|_| EngineError::Timeout(what.to_owned()))?
}
