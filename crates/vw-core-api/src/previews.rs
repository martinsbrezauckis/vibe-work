//! The preview pane in the core: the `preview` command for the UI and the Debug API, the open path the
//! agent tool shares, the change events, and the preview's part of session end and quit. The model is
//! vw-preview's [`PreviewService`]; the native engine comes from the app (`CoreConfig::preview`).
//!
//! Depends on: vw-preview, `preview_ports` (ownership, targets, busy ports), `preview_view`, `core::Shared`.
//! Called by: `dispatch` (`Command::Preview`), `agent_tools` (preview tools), `core` (start,
//! shutdown), `sessions` / `dispatch` (delete, archive).
//! Invariants:
//! - Every URL passes `UrlPolicy::check`. Pinned App previews also check monitored port ownership;
//!   browser web tabs accept local and public addresses without a port lease.
//! - Every origin runs every request directly; the Debug API opens a session's port like the window does (it acts
//!   as the user). `strict` selects agent refusal reporting and pinned App preview port ownership checks.
//! - The change-forwarding task is owned by [`PreviewHub`] and aborted when the hub drops or the core shuts down.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio::task::JoinHandle;
use vw_preview::{
    EngineError, OpenRequest, PreviewConfig, PreviewEngine, PreviewError, PreviewService,
    PreviewView,
};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::preview_ports::{navigation_text, port_owner, session_ports};
#[path = "preview_commands.rs"]
mod commands;
#[path = "preview_maintenance.rs"]
mod maintenance;

#[cfg(test)]
#[path = "previews_readonly_tests.rs"]
mod readonly_tests;

#[cfg(test)]
#[path = "preview_maintenance_tests.rs"]
mod maintenance_tests;

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
    /// policy (link-local, file and app-own targets refused, LAN needs the current grant, names checked before an open
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
    #[cfg(test)]
    maintenance_counts: maintenance_tests::Counts,
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
        #[cfg(test)]
        maintenance_counts: maintenance_tests::Counts::default(),
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
        if let Some(keys) = &shared.config.keys {
            keys.register_browser_guard(guard.clone());
        } else {
            crate::browser_guard::refresh_network_guard(shared);
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
    *hub.ports_task
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(maintenance::start(shared));
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
    let mut result = CoreError::new(code, error.to_string());
    if let PreviewError::ScreenshotHidden(block) = error {
        result.capture_block = Some(block.kind);
    }
    result
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
/// `strict` identifies agent calls for refusal reporting and pinned App preview port ownership.
/// Web tabs use ordinary browser address admission.
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
    let text = navigation_text(url, port, path.as_deref())?;
    let local = crate::browser_local::prepare(shared, &owner, session_id, &text).await?;
    let (shared_for_scan, id) = (shared.clone(), owner.clone());
    let ports = blocking(move || session_ports(&shared_for_scan, &id))
        .await?
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("process monitor: {e}")))?;
    let allow_lan = true;
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
    let (url, destination) = if let Some(url) = local {
        (url, vw_preview::Destination::Public)
    } else {
        hub.service
            .policy()
            .check(
                &text,
                |p| port_owner(shared, &ports, &owner, p, strict),
                allow_lan,
            )
            .map_err(|e| denied(e.to_string()))?
    };
    if session_id == owner
        && let vw_preview::Destination::Loopback { port } = destination
    {
        use vw_preview::{PortOwner, UrlRefusal};
        let reason = match port_owner(shared, &ports, &owner, port, strict) {
            PortOwner::ThisSession => None,
            PortOwner::OtherSession(owner) | PortOwner::OtherApp(owner) => {
                Some(UrlRefusal::NotThisSession { port, owner })
            }
            PortOwner::Nobody => Some(UrlRefusal::Unattributed(port)),
            PortOwner::NotProven => Some(UrlRefusal::NotProven(port)),
        };
        if let Some(reason) = reason {
            return Err(denied(reason.to_string()));
        }
    }
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
    // Preserve the existing named-LAN media-fetch exception as metadata only. A lookup
    // failure never refuses browser navigation; the browser engine resolves independently.
    let lan = destination == vw_preview::Destination::Lan
        || matches!(url.host(), Some(url::Host::Domain(host)) if matches!(hub.service.check_name(host).await, vw_preview::NameVerdict::LocalNetwork(_)));

    let downloads_dir = shared
        .names
        .cwd_of(&shared.store, &owner)
        .map(|cwd| PathBuf::from(cwd).join("downloads"));
    // Other sessions' servers join the URL rules now, from this open's scan, not at the next re-check: a page's
    // first requests to them go out as it loads (macOS compiles the rules before that) and are not repeated.
    let opened = hub
        .service
        .open(OpenRequest {
            session_id: session_id.to_owned(),
            url,
            allowed_ports: ports.allowed(strict),
            downloads_dir,
            wait: true,
            lan,
        })
        .await
        .map_err(|e| core_error(&e))?;
    Ok(opened)
}
