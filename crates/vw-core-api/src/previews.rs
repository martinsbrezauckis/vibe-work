//! The preview pane in the core: the `preview` command for the UI and the Debug API, the open path the
//! agent tool shares, the change events, and the preview's part of session end and quit. The model is
//! vw-preview's [`PreviewService`]; the native engine comes from the app (`CoreConfig::preview`).
//!
//! Depends on: vw-preview, `preview_ports` (ownership, targets, busy ports), `preview_view`, `core::Shared`.
//! Called by: `dispatch` (`Command::Preview`), `agent_tools` (preview tools), `core` (start,
//! shutdown), `sessions` / `dispatch` (delete, archive).
//! Invariants:
//! - Every URL is checked for the session with `UrlPolicy::check` and the monitor's ownership before it is
//!   loaded; refusals name the owner.
//! - Every origin runs every request directly; the Debug API opens a session's port like the window does (it acts
//!   as the user), and the agent's preview tools keep `strict`.
//! - The change-forwarding task is owned by [`PreviewHub`] and aborted when the hub drops or the core shuts down.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio::task::JoinHandle;
use vw_preview::{
    EngineError, HistoryStep, OpenRequest, PreviewConfig, PreviewEngine, PreviewError,
    PreviewService, PreviewView, loopback_url,
};

use crate::command::CommandResult;
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::preview_ports::{busy_ports, busy_view, port_owner, session_ports};
use crate::preview_view::{BrowserOverlayAction, PreviewRequest, PreviewResult};

/// The preview engine and settings the app hands to the core. `None` in `CoreConfig::preview`: no previews (the
/// `preview` command and the agent tools answer that the preview is not available in this build).
#[derive(Clone)]
pub struct PreviewSetup {
    /// The native engine (Tauri child webviews in the app; a fake in tests).
    pub engine: Arc<dyn PreviewEngine>,
    /// Settings (storage root, live cap, dev origin).
    pub config: PreviewConfig,
    /// The agent browser's network guard (the same one the engine sends its webviews through): the
    /// request-time layer. `None` means only that layer is missing: public pages still open under the navigation
    /// policy (link-local, file and app-own targets refused, LAN only for the user, names DNS-checked before an open
    /// and on every in-place navigation, per-session storage).
    pub guard: Option<vw_netguard::NetGuard>,
}

impl std::fmt::Debug for PreviewSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewSetup")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// The running preview service and its event forwarder.
pub(crate) struct PreviewHub {
    pub(crate) service: PreviewService,
    /// The network guard, served while the core runs.
    pub(crate) guard: Option<vw_netguard::NetGuard>,
    /// The agent browser on these previews.
    pub(crate) browser: vw_browser::Browser,
    /// Forwards the agent browser's state changes.
    browser_task: Mutex<Option<JoinHandle<()>>>,
    pending_wipes: PathBuf,
    task: Mutex<Option<JoinHandle<()>>>,
    /// Re-checks the previews' ports.
    ports_task: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for PreviewHub {
    fn drop(&mut self) {
        for slot in [&self.task, &self.browser_task, &self.ports_task] {
            if let Some(task) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
                task.abort();
            }
        }
    }
}

/// Builds the hub (no task yet) and retries the wipes an earlier run could not finish.
pub(crate) fn open(setup: Option<PreviewSetup>) -> Option<Arc<PreviewHub>> {
    let setup = setup?;
    if let Err(error) = vw_platform::ensure_private_dir(&setup.config.storage_root) {
        tracing::warn!(%error, "previews are off: their storage folder could not be prepared");
        return None;
    }
    let pending_wipes = setup.config.storage_root.join("wipe-pending.json");
    retry_wipes(&pending_wipes);
    let service = PreviewService::new(setup.engine, setup.config);
    let browser =
        vw_browser::Browser::new(service.clone(), Arc::new(vw_browser::Sessions::default()));
    Some(Arc::new(PreviewHub {
        service,
        guard: setup.guard,
        browser,
        browser_task: Mutex::new(None),
        pending_wipes,
        task: Mutex::new(None),
        ports_task: Mutex::new(None),
    }))
}

/// Removes folders listed by an earlier run (a deleted session's website data that was still in use then).
fn retry_wipes(list: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(list) else {
        return;
    };
    let paths: Vec<PathBuf> = serde_json::from_str(&text).unwrap_or_default();
    let left: Vec<PathBuf> = paths
        .into_iter()
        .filter(|p| p.exists() && std::fs::remove_dir_all(p).is_err())
        .collect();
    record_wipes(list, &left);
}

fn record_wipes(list: &std::path::Path, left: &[PathBuf]) {
    if left.is_empty() {
        let _ = std::fs::remove_file(list);
        return;
    }
    if let Ok(text) = serde_json::to_vec(left) {
        let temp = list.with_extension("json.new");
        let _ = std::fs::remove_file(&temp);
        if vw_platform::write_private_new(&temp, &text).is_ok() {
            let _ = std::fs::rename(&temp, list);
        }
    }
}

/// Starts forwarding preview changes as `previewChanged` core events, and the network guard: its
/// listeners, the core's loopback answers and Vibe Work's own ports.
#[allow(clippy::too_many_lines)]
pub(crate) fn start_events(shared: &Arc<Shared>) {
    let Some(hub) = &shared.preview else {
        return;
    };
    let weak = Arc::downgrade(shared);
    hub.service.on_guard_refused(move |page, reason| {
        let Some(shared) = weak.upgrade() else {
            return;
        };
        let Some(owner) = shared
            .preview
            .as_ref()
            .and_then(|hub| hub.service.owner_session_id(page))
        else {
            return;
        };
        crate::guard_refusals::record(&shared, &owner, crate::GuardKind::Network, None, reason);
    });
    crate::browser_consent::install(shared);
    crate::browser_delegation::install(shared);
    #[cfg(feature = "e2e-script")]
    if let Ok(origin) = std::env::var("VW_E2E_CONSENT_ORIGIN")
        && let Err(error) = hub.browser.set_test_consent_provider(&origin)
    {
        tracing::warn!(%error, "the consent fixture origin was refused");
    }
    if let Some(guard) = &hub.guard {
        hub.service.set_network_guard(guard.clone());
        guard.set_context(Arc::new(crate::browser_guard::GuardContext::new(shared)));
        crate::browser_guard::register_internal_ports(shared, guard);
        // Public requests go through the user's proxy: Settings > Network (or the process environment) when
        // set, else the guard keeps the system's proxy settings it read when it was bound.
        let network = vw_mistral::network::current();
        if network.has_proxy() {
            guard.set_upstream(vw_netguard::Upstream::from_values(
                network.http_proxy.as_deref(),
                network.https_proxy.as_deref(),
                network.all_proxy.as_deref(),
                network.no_proxy.as_deref(),
            ));
        }
        guard.serve();
    }
    let mut changes = hub.service.changes();
    let weak: Weak<Shared> = Arc::downgrade(shared);
    let task = tokio::spawn(async move {
        loop {
            let page_key = match changes.recv().await {
                Ok(id) => Some(id),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let session_id = page_key.and_then(|page| {
                shared
                    .preview
                    .as_ref()
                    .and_then(|hub| hub.service.owner_session_id(&page))
            });
            if session_id.as_deref() == Some(crate::browser_accounts::ACCOUNT_PAGE) {
                continue;
            }
            shared.emit(CoreEvent::PreviewChanged { session_id });
        }
    });
    *hub.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
    // The agent strip and action log follow `browserChanged`.
    let mut browser_changes = hub.browser.sessions().changes();
    let weak: Weak<Shared> = Arc::downgrade(shared);
    let task = tokio::spawn(async move {
        loop {
            let session_id = match browser_changes.recv().await {
                Ok(id) => Some(id),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let owner = session_id.and_then(|page| {
                shared
                    .preview
                    .as_ref()
                    .and_then(|hub| hub.service.owner_session_id(&page))
            });
            if owner.as_deref() == Some(crate::browser_accounts::ACCOUNT_PAGE) {
                continue;
            }
            shared.emit(CoreEvent::BrowserChanged { session_id: owner });
        }
    });
    *hub.browser_task
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(task);
    // A stopped server or an ended lease leaves its session's preview ports within
    // `PORTS_RECHECK`, so the URL rules and the request checks drop it promptly (an open adds ports; this only
    // withdraws), and the URL rules follow every session's servers. One process scan per round, only while
    // some preview is mounted or has ports.
    let weak: Weak<Shared> = Arc::downgrade(shared);
    let task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(PORTS_RECHECK).await;
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let Some(hub) = shared.preview.clone() else {
                return;
            };
            if let Err(error) = hub.service.reap_idle_browsers().await {
                tracing::debug!(%error, "idle browser release failed");
            }
            let sessions = hub.service.sessions_with_ports();
            if sessions.is_empty() && hub.service.mount_labels().is_empty() {
                continue;
            }
            let mut owners: Vec<String> = sessions
                .iter()
                .filter_map(|page| hub.service.owner_session_id(page))
                .collect();
            owners.sort_unstable();
            owners.dedup();
            let scan = {
                let (owned, list) = (shared.clone(), owners);
                tokio::task::spawn_blocking(move || crate::preview_ports::live_ports(&owned, &list))
                    .await
            };
            match scan {
                Ok(Ok(live)) => {
                    for page in &sessions {
                        if let Some(owner) = hub.service.owner_session_id(page) {
                            if let Some(port) = hub.service.local_server_port(page) {
                                let listening = live
                                    .listeners
                                    .get(&owner)
                                    .is_some_and(|ports| ports.contains(&port));
                                let _ = hub
                                    .service
                                    .set_local_server_listening(page, port, listening);
                            }
                            let current = live.sessions.get(&owner).cloned().unwrap_or_default();
                            let _ = hub.service.retain_allowed_ports(page, &current).await;
                        }
                    }
                    hub.service.set_server_ports(live.servers).await;
                }
                // A failed scan withdraws nothing (the request-time checks still ask the owners themselves).
                Ok(Err(error)) => tracing::debug!(%error, "preview port re-check skipped"),
                Err(error) => tracing::debug!(%error, "preview port re-check failed"),
            }
        }
    });
    *hub.ports_task
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(task);
}

/// How often the previews' ports are re-checked.
const PORTS_RECHECK: std::time::Duration = std::time::Duration::from_secs(3);

/// A session ended: close its preview; on delete wipe its website data (leftovers retried next start).
pub(crate) async fn session_ended(shared: &Shared, session_id: &str, deleted: bool) {
    let Some(hub) = &shared.preview else {
        return;
    };
    let web_tabs = hub.service.web_tabs(session_id);
    for page in &web_tabs {
        hub.browser.sessions().cancel_actions(page);
        hub.browser.clear_secret_choice(page);
        if deleted {
            hub.browser.sessions().session_ended(page);
        }
    }
    if deleted {
        hub.browser.sessions().session_ended(session_id);
    }
    let left = hub.service.session_ended(session_id, deleted).await;
    hub.browser.clear_secret_choice(session_id);
    if deleted && let Err(error) = hub.browser.forget_delegation_receipts(session_id) {
        tracing::warn!(%error, session_id, "deleted session's browser receipts could not be removed");
    }
    if !left.is_empty() {
        tracing::info!(
            count = left.len(),
            "preview data kept until the next start (still in use)"
        );
        let mut all: Vec<PathBuf> = std::fs::read_to_string(&hub.pending_wipes)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        all.extend(left);
        record_wipes(&hub.pending_wipes, &all);
    }
}

/// Closes every preview (quit) and stops the forwarder.
pub(crate) async fn shutdown(shared: &Shared) {
    if let Some(hub) = &shared.preview {
        for slot in [&hub.task, &hub.browser_task, &hub.ports_task] {
            if let Some(task) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
                task.abort();
            }
        }
        hub.service.shutdown().await;
        if let Some(guard) = &hub.guard {
            guard.shutdown();
        }
    }
}

/// Maps a preview failure to the core's codes.
pub(crate) fn core_error(error: &PreviewError) -> CoreError {
    let code = match error {
        PreviewError::NothingOpen | PreviewError::Engine(EngineError::NotMounted) => {
            ErrorCode::NotFound
        }
        PreviewError::ScreenshotHidden(_)
        | PreviewError::BrowserBusy
        | PreviewError::Refused(_)
        | PreviewError::Engine(EngineError::FocusRequired | EngineError::Unsupported(_)) => {
            ErrorCode::Denied
        }
        PreviewError::Layout(_)
        | PreviewError::PoolSetting(_)
        | PreviewError::InvalidTab(_)
        | PreviewError::NotInView => ErrorCode::InvalidParams,
        PreviewError::Engine(EngineError::Timeout(_)) => ErrorCode::Timeout,
        PreviewError::Engine(EngineError::Platform(_))
        | PreviewError::Image(_)
        | PreviewError::PoolStorage(_) => ErrorCode::Internal,
    };
    CoreError::new(code, error.to_string())
}

fn unavailable() -> CoreError {
    CoreError::not_in_this_build("the preview is not available in this build of Vibe Work")
}

pub(crate) fn hub(shared: &Shared) -> Result<&PreviewHub, CoreError> {
    shared.preview.as_deref().ok_or_else(unavailable)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("preview task failed: {e}")))
}

/// Opens a URL (or a port + path) of `session_id` in its preview: the shared path of the UI, the Debug API and
/// the agent's `preview_open` / `browser_act navigate`. Refusals are plain text naming the owner.
/// `strict` means the AGENT is opening: only proven ports (fact-tier ownership or a lease) open, the preview may then
/// navigate only among those, and nothing on the local network opens. Otherwise the user (the
/// address bar, or the Debug API acting as the user) may open a private-network address or a name that
/// resolves there; it opens as a Lan page the agent neither reads nor acts on.
pub(crate) async fn open_for_session(
    shared: &Arc<Shared>,
    session_id: &str,
    target: (Option<String>, Option<u16>, Option<String>),
    strict: bool,
) -> Result<PreviewView, CoreError> {
    let (url, port, path) = target;
    let hub = hub(shared)?;
    let owner = hub.service.owner_session_id(session_id).ok_or_else(|| {
        CoreError::new(
            ErrorCode::Denied,
            "browser page is not owned by a loaded session",
        )
    })?;
    if !crate::new_chat::known_owner(shared, &owner) {
        return Err(CoreError::not_loaded(&owner));
    }
    let text = match (url, port) {
        (Some(url), None) => url,
        (None, Some(port)) => loopback_url(port, path.as_deref())
            .map_err(|e| CoreError::new(ErrorCode::InvalidParams, e.to_string()))?,
        _ => {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "give either a url or a port",
            ));
        }
    };
    let (shared_for_scan, id) = (shared.clone(), owner.clone());
    let ports = blocking(move || session_ports(&shared_for_scan, &id))
        .await?
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("process monitor: {e}")))?;
    let user = !strict;
    let denied = |text: String| {
        let message = if strict {
            crate::guard_refusals::record(
                shared,
                &owner,
                crate::GuardKind::Network,
                crate::guard_refusals::step(shared, &owner, None),
                &text,
            )
        } else {
            text
        };
        CoreError::new(ErrorCode::Denied, message)
    };
    let (url, destination) = hub
        .service
        .policy()
        .check(
            &text,
            |p| port_owner(shared, &ports, &owner, p, strict),
            user,
        )
        .map_err(|e| denied(e.to_string()))?;
    if session_id == owner
        && !matches!(
            destination,
            vw_preview::Destination::Loopback { .. } | vw_preview::Destination::Blank
        )
    {
        return Err(denied(
            "App preview opens this session's local servers only; open a web tab for public pages"
                .to_owned(),
        ));
    }
    let lan = match destination {
        vw_preview::Destination::Lan => true,
        vw_preview::Destination::Public => match names_public(hub, &url).await? {
            Points::Public => false,
            Points::LocalNetwork if user => true,
            Points::LocalNetwork => {
                return Err(denied(
                    vw_preview::UrlRefusal::Lan(
                        url.host_str().unwrap_or("").chars().take(80).collect(),
                    )
                    .to_string(),
                ));
            }
        },
        _ => false,
    };
    let downloads_dir = shared
        .names
        .cwd_of(&shared.store, &owner)
        .map(|cwd| PathBuf::from(cwd).join("downloads"));
    // Other sessions' servers join the URL rules now, from this open's scan, not at the next re-check: a page's
    // first requests to them go out as it loads (macOS compiles the rules before that) and are not repeated.
    hub.service.set_server_ports(ports.servers.clone()).await;
    hub.service
        .open(OpenRequest {
            session_id: session_id.to_owned(),
            url,
            allowed_ports: ports.allowed(strict),
            downloads_dir,
            wait: true,
            lan,
        })
        .await
        .map_err(|e| core_error(&e))
}

/// Where a public-looking host name points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Points {
    /// Every answer is public.
    Public,
    /// Some answers are private-network or carrier-grade-NAT addresses (the rest public): a local-network page.
    LocalNetwork,
}

/// Where a host NAME points (the baseline every platform has; Linux checks every request again in the
/// network guard), through the preview service's shared, bounded lookup (the navigation hooks then answer this page
/// from its cache). A lookup that fails or times out refuses the open (fail closed: the page would not load anyway).
/// Loopback, link-local and other special answers refuse it for everyone; private-network answers make it a
/// local-network page (the caller decides who may open those).
async fn names_public(hub: &PreviewHub, url: &url::Url) -> Result<Points, CoreError> {
    let Some(url::Host::Domain(name)) = url.host() else {
        return Ok(Points::Public);
    };
    match hub.service.check_name(name).await {
        vw_preview::NameVerdict::Public => Ok(Points::Public),
        vw_preview::NameVerdict::LocalNetwork(_) => Ok(Points::LocalNetwork),
        vw_preview::NameVerdict::Refused(why) => Err(CoreError::new(
            ErrorCode::Denied,
            format!("{why}: not opened"),
        )),
    }
}

impl Core {
    /// Executes one preview request (the same for every origin).
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn preview(
        &self,
        request: PreviewRequest,
    ) -> Result<CommandResult, CoreError> {
        let shared = &self.shared;
        let hub = hub(shared)?;
        let service = &hub.service;
        let err = |e: PreviewError| core_error(&e);
        let result = match request {
            PreviewRequest::List { session_id } => self.preview_list(&session_id).await?,
            PreviewRequest::Open {
                session_id,
                url,
                port,
                path,
            } => PreviewResult::Preview {
                preview: Some(
                    open_for_session(shared, &session_id, (url, port, path), false).await?,
                ),
            },
            PreviewRequest::Back { session_id } => {
                service
                    .history(&session_id, HistoryStep::Back)
                    .await
                    .map_err(err)?;
                PreviewResult::Preview {
                    preview: service.view(&session_id),
                }
            }
            PreviewRequest::Forward { session_id } => {
                service
                    .history(&session_id, HistoryStep::Forward)
                    .await
                    .map_err(err)?;
                PreviewResult::Preview {
                    preview: service.view(&session_id),
                }
            }
            PreviewRequest::Reload { session_id } => {
                if let Some(port) = service.local_server_port(&session_id) {
                    let (shared_for_scan, owner) = (shared.clone(), session_id.clone());
                    let ports = blocking(move || session_ports(&shared_for_scan, &owner))
                        .await?
                        .map_err(|e| {
                            CoreError::new(ErrorCode::Internal, format!("process monitor: {e}"))
                        })?;
                    let listening = ports.targets.iter().any(|t| t.port == port && t.listening);
                    let was_stopped = service
                        .view(&session_id)
                        .is_some_and(|v| v.state == vw_preview::PreviewState::Stopped);
                    let _ = service.set_local_server_listening(&session_id, port, listening);
                    if listening && was_stopped {
                        let url = service.view(&session_id).and_then(|v| v.url);
                        if let Some(url) = url {
                            open_for_session(shared, &session_id, (Some(url), None, None), false)
                                .await?;
                        }
                    } else if listening {
                        service
                            .history(&session_id, HistoryStep::Reload)
                            .await
                            .map_err(err)?;
                    }
                } else {
                    service
                        .history(&session_id, HistoryStep::Reload)
                        .await
                        .map_err(err)?;
                }
                PreviewResult::Preview {
                    preview: service.view(&session_id),
                }
            }
            PreviewRequest::SetWidth {
                session_id,
                tab_id,
                width,
            } => {
                if !crate::new_chat::known_owner(shared, &session_id) {
                    return Err(CoreError::not_loaded(&session_id));
                }
                let page = crate::browser_tabs::user_page(
                    service,
                    &session_id,
                    Some(tab_id.as_deref().unwrap_or(&session_id)),
                )?;
                service.set_width(&page, width).await.map_err(err)?;
                PreviewResult::Preview {
                    preview: service.view(&page),
                }
            }
            PreviewRequest::Layout {
                session_id,
                tab_id,
                rect,
            } => {
                if let Some(owner) = session_id.as_deref()
                    && !crate::new_chat::known_owner(shared, owner)
                {
                    return Err(CoreError::not_loaded(owner));
                }
                let page = match (session_id.as_deref(), tab_id) {
                    (None, None) => None,
                    (Some(owner), None) => Some(owner.to_owned()),
                    (Some(owner), Some(key)) => {
                        if service.owner_session_id(&key).as_deref() != Some(owner) {
                            return Err(CoreError::new(
                                ErrorCode::Denied,
                                "tab does not belong to this session",
                            ));
                        }
                        Some(key)
                    }
                    (None, Some(_)) => {
                        return Err(CoreError::new(
                            ErrorCode::InvalidParams,
                            "tabId needs sessionId",
                        ));
                    }
                };
                service.report_tab_layout(page, rect).await.map_err(err)?;
                PreviewResult::Done {}
            }
            PreviewRequest::Visibility { report } => {
                service.report_view(report, |id| crate::new_chat::is_draft(shared, id));
                PreviewResult::Done {}
            }
            PreviewRequest::NativeWindowVisibility { visible } => {
                service.report_native_window_visibility(visible);
                PreviewResult::Done {}
            }
            PreviewRequest::BrowserOverlay {
                session_id,
                tab_id,
                action,
            } => {
                if !crate::new_chat::known_owner(shared, &session_id) {
                    return Err(CoreError::not_loaded(&session_id));
                }
                let page = match tab_id {
                    Some(key) => {
                        if service.owner_session_id(&key).as_deref() != Some(session_id.as_str()) {
                            return Err(CoreError::new(
                                ErrorCode::Denied,
                                "tab does not belong to this session",
                            ));
                        }
                        key
                    }
                    None => session_id.clone(),
                };
                tracing::debug!(session = %session_id, page = %page, ?action, "browser overlay request");
                match action {
                    BrowserOverlayAction::Prepare => {
                        match service.prepare_browser_overlay(&session_id, &page).await {
                            Ok(prepared) => PreviewResult::Overlay {
                                snapshot: prepared.snapshot,
                                bounds: Some(prepared.bounds),
                            },
                            // A tab switch overtook the menu: nothing to park, the window closes its menus.
                            Err(PreviewError::NotInView) => PreviewResult::Overlay {
                                snapshot: None,
                                bounds: None,
                            },
                            Err(error) => return Err(err(error)),
                        }
                    }
                    BrowserOverlayAction::Show => {
                        service
                            .show_browser_overlay(&session_id, &page)
                            .await
                            .map_err(err)?;
                        PreviewResult::Overlay {
                            snapshot: None,
                            bounds: None,
                        }
                    }
                    BrowserOverlayAction::Close => {
                        service
                            .close_browser_overlay(&session_id, &page)
                            .await
                            .map_err(err)?;
                        PreviewResult::Overlay {
                            snapshot: None,
                            bounds: None,
                        }
                    }
                }
            }
            PreviewRequest::Close { session_id } => {
                service.close(&session_id).await.map_err(err)?;
                PreviewResult::Preview {
                    preview: service.view(&session_id),
                }
            }
            PreviewRequest::Screenshot { session_id } => {
                let shot = service.screenshot(&session_id).await.map_err(err)?;
                PreviewResult::Screenshot {
                    png_base64: shot.base64(),
                    width: shot.width,
                    height: shot.height,
                }
            }
            PreviewRequest::Console { session_id, limit } => {
                let limit = limit.and_then(|l| usize::try_from(l).ok());
                let (entries, dropped) = service.console(&session_id, limit).await.map_err(err)?;
                PreviewResult::Console { entries, dropped }
            }
            PreviewRequest::Explain { session_id, port } => PreviewResult::Explanation {
                explanation: self.explain_for_preview(&session_id, port).await?,
            },
            PreviewRequest::StopOwner { session_id, port } => {
                self.stop_busy_owner(&session_id, port).await?;
                PreviewResult::Done {}
            }
            PreviewRequest::Pick {
                session_id,
                target,
                tab_id,
            } => PreviewResult::Picked {
                picked: crate::preview_pick::run(service, &session_id, target, tab_id).await?,
            },
            PreviewRequest::CancelPick { session_id } => {
                crate::preview_pick::cancel(service, &session_id).await;
                PreviewResult::Done {}
            }
            PreviewRequest::CopyText {
                session_id,
                tab_id,
                target,
            } => PreviewResult::Copied {
                text: crate::preview_pick::copy_text(service, &session_id, &tab_id, &target)
                    .await?,
            },
        };
        Ok(CommandResult::Preview { result })
    }

    async fn preview_list(&self, session_id: &str) -> Result<PreviewResult, CoreError> {
        if !crate::new_chat::known_owner(&self.shared, session_id) {
            return Err(CoreError::not_loaded(session_id));
        }
        let hub = hub(&self.shared)?;
        let (shared, id) = (self.shared.clone(), session_id.to_owned());
        let (ports, busy) = blocking(move || {
            session_ports(&shared, &id).map(|ports| (ports, busy_ports(&shared, &id)))
        })
        .await?
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("process monitor: {e}")))?;
        if let Some(port) = hub.service.local_server_port(session_id) {
            let listening = ports.targets.iter().any(|t| t.port == port && t.listening);
            let _ = hub
                .service
                .set_local_server_listening(session_id, port, listening);
        }
        // A list from the window keeps what the user may navigate to; it never widens an agent-opened preview's
        // set beyond proven ports, because the set is replaced only by an open.
        Ok(PreviewResult::Previews {
            preview: hub.service.view(session_id),
            targets: ports.targets,
            leased_ports: ports.leased,
            busy,
        })
    }

    async fn explain_for_preview(
        &self,
        session_id: &str,
        port: u16,
    ) -> Result<crate::preview_view::BusyPortView, CoreError> {
        let explanation = self.explain_port(Some(session_id.to_owned()), port).await?;
        Ok(busy_view(&self.shared, session_id, &explanation, None))
    }
}

#[cfg(test)]
#[path = "previews_readonly_tests.rs"]
mod readonly_tests;
