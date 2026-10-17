//! The project handbook in the core: the `pages` command for the UI and the Debug API, the change
//! events, the viewer's file-change reload, and the handbook part of session end and quit. Hub and overview reads
//! are in `pages_overview`, stored data in `pages_data`. The model is vw-pages' [`PagesService`];
//! the viewer engine comes from the app (`CoreConfig::pages`).
//!
//! Depends on: vw-pages, `pages_view`, `pages_overview`, `pages_data`, `core::Shared`, `names` (a session's folder),
//! `plan_profile`.
//! Called by: `dispatch` (`Command::Pages`), `core` (start, shutdown), `dispatch`/`sessions` (delete, archive).
//! Invariants:
//! - A session's project is its own folder (`Names::cwd_of`); clients never name a folder.
//! - Every origin runs every request directly (the Debug API acts as the user; the dialog is gone).
//! - A comment's section must exist in the page FILE when it is added (never trusted from the client).
//! - The forwarding/reload task is owned by [`PagesHub`] and aborted on drop or shutdown.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use tokio::task::JoinHandle;
use vw_pages::{PagePath, PagesConfig, PagesError, PagesService};
use vw_preview::{EngineError, HistoryStep, PreviewEngine};

use crate::command::CommandResult;
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::pages_data::page_result;
use crate::pages_view::{PagesRequest, PagesResult};

/// How often the shown page's file is checked for changes (the viewer reloads when the agent rewrote it).
const RELOAD_TICK: Duration = Duration::from_millis(1500);

/// The viewer engine and settings the app hands to the core. `None` in `CoreConfig::pages`: no Project Pages (the
/// `pages` command answers that they are not available in this build).
#[derive(Clone)]
pub struct PagesSetup {
    /// The viewer engine (a second Tauri engine instance in the app; a fake in tests).
    pub engine: Arc<dyn PreviewEngine>,
    /// Settings (app-data folders, timeouts).
    pub config: PagesConfig,
    /// The visual checker's engine (a hidden webview of its own; `None`: no visual check in this build).
    pub checker: Option<Arc<dyn PreviewEngine>>,
}

impl std::fmt::Debug for PagesSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PagesSetup")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// The running service and its tasks.
pub(crate) struct PagesHub {
    pub(crate) service: PagesService,
    task: Mutex<Option<JoinHandle<()>>>,
    /// The background visual checks: at most one task at a time.
    pub(crate) checks: Mutex<Option<JoinHandle<()>>>,
    /// Plan files the plan checklist shows, by session, with the modification time last seen.
    pub(crate) plans:
        Mutex<std::collections::HashMap<String, (PathBuf, Option<std::time::SystemTime>)>>,
}

impl Drop for PagesHub {
    fn drop(&mut self) {
        self.stop_task();
    }
}

/// Builds the hub (no task yet).
pub(crate) fn open(setup: Option<PagesSetup>) -> Option<Arc<PagesHub>> {
    let setup = setup?;
    if let Err(error) = vw_platform::ensure_private_dir(&setup.config.data_dir) {
        tracing::warn!(%error, "Project Pages are off: their data folder could not be prepared");
        return None;
    }
    let storage = setup.config.view_storage.join("check");
    let mut service = PagesService::new(setup.engine, setup.config);
    if let Some(engine) = setup.checker {
        service = service.with_checker(engine, storage);
    }
    Some(Arc::new(PagesHub {
        service,
        task: Mutex::new(None),
        checks: Mutex::new(None),
        plans: Mutex::new(std::collections::HashMap::new()),
    }))
}

/// Starts forwarding viewer changes as `pagesChanged` events and the file-change reload tick.
pub(crate) fn start_events(shared: &Arc<Shared>) {
    if let Some(hub) = &shared.pages {
        start_hub_events(shared, hub, |session_id| CoreEvent::PagesChanged {
            session_id,
        });
    }
}

/// Forwards one viewer service's changes as the events `make` builds, and runs its reload tick (the Project tab's
/// hub and the File tab's viewer). The task is owned by `hub`.
pub(crate) fn start_hub_events(
    shared: &Arc<Shared>,
    hub: &Arc<PagesHub>,
    make: fn(Option<String>) -> CoreEvent,
) {
    let mut changes = hub.service.changes();
    let service = hub.service.clone();
    let weak: Weak<Shared> = Arc::downgrade(shared);
    let task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(RELOAD_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                change = changes.recv() => {
                    let session_id = match change {
                        Ok(id) => id,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    };
                    let Some(shared) = weak.upgrade() else { return };
                    shared.emit(make(session_id));
                }
                _ = tick.tick() => {
                    service.reload_if_changed().await;
                    let Some(shared) = weak.upgrade() else { return };
                    crate::pages_plan::plan_tick(&shared);
                }
            }
        }
    });
    *hub.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
}

impl PagesHub {
    /// Stops the event and reload task and the background checks (quit).
    pub(crate) fn stop_task(&self) {
        for slot in [&self.task, &self.checks] {
            if let Some(task) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
                task.abort();
            }
        }
    }
}

/// A session ended: its viewer goes.
pub(crate) async fn session_ended(shared: &Shared, session_id: &str) {
    if let Some(hub) = &shared.pages {
        hub.service.session_ended(session_id).await;
        hub.plans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session_id);
    }
}

/// Closes the viewer and stops the page servers (quit).
pub(crate) async fn shutdown(shared: &Shared) {
    if let Some(hub) = &shared.pages {
        hub.stop_task();
        hub.service.shutdown().await;
        hub.service.shutdown_checker().await;
    }
}

/// Maps a Pages failure to the core's codes.
pub(crate) fn core_error(error: &PagesError) -> CoreError {
    let code = match error {
        PagesError::NoPagesFolder(_)
        | PagesError::NoPage(_)
        | PagesError::NotShown
        | PagesError::Comment(vw_pages::CommentError::NotFound)
        | PagesError::Engine(EngineError::NotMounted) => ErrorCode::NotFound,
        PagesError::Path(_) | PagesError::NoSection | PagesError::NothingPicked => {
            ErrorCode::InvalidParams
        }
        PagesError::Comment(vw_pages::CommentError::Invalid(_))
        | PagesError::Answer(vw_pages::AnswerError::Invalid(_)) => ErrorCode::InvalidParams,
        PagesError::Engine(EngineError::Unsupported(_) | EngineError::FocusRequired) => {
            ErrorCode::Denied
        }
        PagesError::Engine(EngineError::Timeout(_)) => ErrorCode::Timeout,
        PagesError::Engine(EngineError::Platform(_))
        | PagesError::Comment(vw_pages::CommentError::Io(_))
        | PagesError::Answer(vw_pages::AnswerError::Io(_))
        | PagesError::Other(_) => ErrorCode::Internal,
    };
    CoreError::new(code, error.to_string())
}

fn hub(shared: &Shared) -> Result<&PagesHub, CoreError> {
    shared.pages.as_deref().ok_or_else(|| {
        CoreError::not_in_this_build("Project Pages are not available in this build of Vibe Work")
    })
}

/// an anchor to scroll to must be a section or step id, checked before anything opens.
fn check_anchor(anchor: Option<&str>) -> Result<(), CoreError> {
    match anchor {
        Some(a) if a.len() > 100 || !vw_pages::handbook::format::is_id(a) => Err(CoreError::new(
            ErrorCode::InvalidParams,
            "anchor must be a section or step id (a-z, 0-9 and -)",
        )),
        _ => Ok(()),
    }
}

pub(crate) fn path_of(text: &str) -> Result<PagePath, CoreError> {
    PagePath::parse(text).map_err(|e| CoreError::new(ErrorCode::InvalidParams, e.to_string()))
}

pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("pages task failed: {e}")))
}

/// The last folder name of a project.
pub(crate) fn name_of(project: &Path) -> String {
    project.file_name().map_or_else(
        || project.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl Core {
    /// The folder whose handbook `session_id` shows (Project tab, comments, viewer): its project's main folder, or its
    /// own folder when it works there, in a copy of it, or has no project with a main folder
    /// (`projects_filing::handbook_folder`).
    pub(crate) fn project_of(&self, session_id: &str) -> Result<PathBuf, CoreError> {
        self.shared
            .names
            .cwd_of(&self.shared.store, session_id)
            // The session's project's main folder, unless it works there or in a copy of it.
            .map(|cwd| crate::projects_filing::handbook_folder(&self.shared, session_id, &cwd))
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::NotFound,
                    format!(
                        "session {session_id} has no folder Vibe Work knows (list or open it first)"
                    ),
                )
            })
    }

    /// Executes one Pages request (the same for every origin; only `exportPage` with a `destination`
    /// is the Debug API's: the window saves through the native save dialog).
    pub(crate) async fn pages(
        &self,
        origin: crate::command::Origin,
        request: PagesRequest,
    ) -> Result<CommandResult, CoreError> {
        let shared = &self.shared;
        let service = hub(shared)?.service.clone();
        let err = |e: PagesError| core_error(&e);
        let result = match request {
            PagesRequest::Hub { session_id } => self.pages_hub(&service, &session_id).await?,
            PagesRequest::Overview {} => self.overview(&service).await?,
            PagesRequest::ProjectHub { project } => self.project_hub(&service, &project).await?,
            PagesRequest::Page { session_id, path } => {
                let project = self.project_of(&session_id)?;
                page_result(&service, &session_id, &project, path_of(&path)?).await?
            }
            PagesRequest::Open {
                session_id,
                path,
                anchor,
            } => {
                check_anchor(anchor.as_deref())?;
                let project = self.project_of(&session_id)?;
                let path = path.as_deref().map(path_of).transpose()?;
                // A page about to show gets its visual check first in the queue.
                if let Some(page) = &path {
                    crate::pages_look::queue_checks(&self.shared, &project, vec![page.to_string()]);
                }
                PagesResult::Viewer {
                    viewer: Some(
                        service
                            .open_at(&session_id, &project, path, anchor)
                            .await
                            .map_err(err)?,
                    ),
                }
            }
            PagesRequest::Plan { session_id } => {
                let project = self.project_of(&session_id)?;
                let (owned, folder) = (service.clone(), project.clone());
                let plan = blocking(move || owned.current_plan(&folder))
                    .await?
                    .ok_or_else(|| {
                        CoreError::new(
                            ErrorCode::NotFound,
                            "this project has no plan page yet (plans/ in the pages folder)",
                        )
                    })?;
                service
                    .open(&session_id, &project, Some(plan.clone()))
                    .await
                    .map_err(err)?;
                page_result(&service, &session_id, &project, plan).await?
            }
            PagesRequest::Back { session_id } => {
                self.step(&service, &session_id, HistoryStep::Back).await?
            }
            PagesRequest::Forward { session_id } => {
                self.step(&service, &session_id, HistoryStep::Forward)
                    .await?
            }
            PagesRequest::Reload { session_id } => {
                self.step(&service, &session_id, HistoryStep::Reload)
                    .await?
            }
            PagesRequest::Layout { session_id, rect } => {
                service.layout(session_id.zip(rect)).await.map_err(err)?;
                PagesResult::Done {}
            }
            PagesRequest::PickSection {
                session_id,
                section,
            } => {
                self.pages_pick(&service, &session_id, section.as_deref())
                    .await?
            }
            PagesRequest::CancelPick {} => {
                service.cancel_pick().await;
                PagesResult::Done {}
            }
            PagesRequest::ClearExternal { session_id } => {
                service.clear_external(&session_id);
                PagesResult::Viewer {
                    viewer: service.view(&session_id),
                }
            }
            PagesRequest::Screenshot { session_id } => {
                let shot = service.screenshot(&session_id).await.map_err(err)?;
                PagesResult::Screenshot {
                    png_base64: shot.base64(),
                    width: shot.width,
                    height: shot.height,
                }
            }
            other => self.pages_more(&service, origin, other).await?,
        };
        Ok(CommandResult::Pages { result })
    }

    /// `pickSection`: waits for a person's click, or with `section` (the Debug API's way: nobody clicks) reads the
    /// named section from the page file at once.
    async fn pages_pick(
        &self,
        service: &PagesService,
        session_id: &str,
        section: Option<&str>,
    ) -> Result<PagesResult, CoreError> {
        let project = self.project_of(session_id)?;
        let picked = match section {
            Some(name) => {
                let name = crate::preview_pick::section_name(Some(name))?;
                service.section_named(session_id, &project, name).await
            }
            None => service.pick_section(session_id, &project).await,
        };
        Ok(PagesResult::Picked {
            picked: picked.map_err(|e| core_error(&e))?,
        })
    }

    async fn step(
        &self,
        service: &PagesService,
        session_id: &str,
        step: HistoryStep,
    ) -> Result<PagesResult, CoreError> {
        service
            .history(session_id, step)
            .await
            .map_err(|e| core_error(&e))?;
        Ok(PagesResult::Viewer {
            viewer: service.view(session_id),
        })
    }
}
