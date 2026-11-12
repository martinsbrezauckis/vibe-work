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

use crate::engine::{EngineError, EngineFuture, HistoryStep, PreviewEngine, Storage};
use crate::engine_pool::PoolState;
use crate::hooks::{Hooks, Inner};
use crate::layout::{self, DeviceWidth};
use crate::policy::UrlPolicy;
pub use crate::preview_error::PreviewError;
use crate::state::{BrowserMountState, PreviewView, State};
use crate::visibility::VisibilityReport;

#[path = "service_accounts.rs"]
mod service_accounts;
#[path = "service_adopt.rs"]
mod service_adopt;
#[path = "service_capture.rs"]
mod service_capture;
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
#[path = "service_shields.rs"]
mod service_shields;
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
    /// How host names are looked up for backend media scope metadata (`None`: the system resolver;
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
    pub(crate) actions: Arc<crate::action_tracking::ActionTracker>,
    pub(crate) inner: Arc<Inner>,
    pub(crate) engine: Arc<dyn PreviewEngine>,
    config: Arc<PreviewConfig>,
    pool: Arc<std::sync::Mutex<PoolState>>,
    pool_op: Arc<tokio::sync::Mutex<()>>,
    /// Sessions that took over a New chat screen's website data (`service_adopt`).
    adoptions: Arc<std::sync::Mutex<service_adopt::Adoptions>>,
    activity: Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>,
        >,
    >,
    pub(crate) layout_op: Arc<tokio::sync::Mutex<()>>,
    /// Refuses placements of a replaced main document; checked and advanced under `layout_op`.
    pub(crate) placement_gate: Arc<crate::PlacementGate>,
    /// The window's last report, with whether its selection is the New chat screen's owner.
    pub(crate) visibility: Arc<std::sync::Mutex<Option<(VisibilityReport, bool)>>>,
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
            Err(PreviewError::ScreenshotHidden(reason)) => Some(reason.text),
            _ => None,
        }
    }

    /// A service over `engine` (its hooks are attached here).
    #[must_use]
    pub fn new(engine: Arc<dyn PreviewEngine>, config: PreviewConfig) -> Self {
        let inner = Arc::new(Inner {
            window_close: std::sync::Mutex::new(None),
            popup: std::sync::Mutex::new(None),
            shields: std::sync::Mutex::new(crate::shields::Saved::load(&config.storage_root)),
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
        let adoptions = service_adopt::Adoptions::load(&config.storage_root);
        Self {
            actions: Arc::default(),
            adoptions: Arc::new(std::sync::Mutex::new(adoptions)),
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
                    "could not check where {host} points (the lookup failed), so it was not opened; try again"
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

    /// The storage of a session's preview: its owner's, or the New chat screen's that the owner adopted at its first
    /// message (`adopt_storage`).
    #[must_use]
    pub fn storage(&self, session_id: &str) -> Storage {
        let owner = self.inner.lock().owner_of(session_id);
        let key = self.storage_key_of(&owner);
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
                .and_then(|s| layout::place_viewport(s.area, entry.width, entry.viewport).ok());
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
            entry.viewport = None;
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

    /// Evaluates `script` natively in the session's preview (mounted again first when suspended) and answers the
    /// JSON of its value: untrusted page data (the element picker).
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], engine failures and timeouts.
    pub async fn evaluate(&self, session_id: &str, script: String) -> Result<String, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        let id = self.ensure_mounted(session_id).await?;
        Ok(self
            .bounded("evaluate", self.engine.evaluate_isolated(&id, script))
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
        self.close_page_window(session_id).await?;
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
