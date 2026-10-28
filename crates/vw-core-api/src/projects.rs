//! The `projects` command: the user's project groups (named projects, an optional main folder, the
//! user's order, the session -> project map). The store and the rules are in vw-projects; filing and the sweep in
//! `projects_filing`.
//!
//! Depends on: vw-projects, `projects_filing` (store access, sweep), `projects_view` (wire types), vw-gate (picked
//! folders). Called by: `dispatch` (`Command::Projects`), Host (saved-main-folder reveal admission).
//! Invariants:
//! - Both clients act alike (the Debug API controls the app as the user; no native dialog, no
//!   redaction for project requests). The origin is still logged with every failure by `Core::execute`.
//! - A main folder must be a folder a known session works in, a project's main folder already, or one the user
//!   picked in the native dialog, for BOTH origins. The Debug API gets no wider rule: a `ui/projects` request's
//!   window commands run as the window's, and a folder it names becomes usable through the Debug API's `pickFolder`
//!   (the request's grant, `vw-host` `ui_grants`), so the rule stays one for both; a main folder reaches the
//!   `vw-plan` write profile and the page server (covered by `vw_core_projects`).
//! - `delete` never deletes a session: its members move to "not in a project".

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::Ordering;

use serde_json::json;
use vw_projects::{MAX_MOVE, Project};

use crate::command::{CommandResult, Origin};
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::projects_filing::{sweep, sweep_once, update};
use crate::projects_view::{ProjectView, ProjectsRequest, ProjectsResult};

fn view(project: &Project, sessions: usize) -> ProjectView {
    ProjectView {
        id: project.id.clone(),
        name: project.name.clone(),
        main_folder: project.main_folder.clone(),
        folder_name: project
            .main_folder
            .as_deref()
            .map(vw_projects::folder_label),
        auto: project.auto,
        sessions: u32::try_from(sessions).unwrap_or(u32::MAX),
    }
}

fn list(shared: &Shared) -> ProjectsResult {
    let swept = shared.projects.swept.load(Ordering::Relaxed);
    shared.projects.store.read(|s| {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for project in s.members().values().flatten() {
            *counts.entry(project.as_str()).or_default() += 1;
        }
        let projects = s
            .projects()
            .iter()
            .map(|p| view(p, counts.get(p.id.as_str()).copied().unwrap_or(0)))
            .collect();
        ProjectsResult::List {
            projects,
            members: s.members().clone(),
            swept,
        }
    })
}

fn not_found(id: &str) -> CoreError {
    CoreError::new(
        ErrorCode::NotFound,
        format!("there is no project with the id {id:.40}"),
    )
}

/// Refuses a main folder the origin may not name (see the module invariants).
fn check_folder(shared: &Shared, folder: &str) -> Result<(), CoreError> {
    let path = Path::new(folder);
    if !path.is_absolute() {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "the main folder must be an absolute path",
        ));
    }
    let known = shared.gate.is_picked_folder(path)
        || shared.names.is_session_folder(&shared.store, folder)
        || shared.projects.store.read(|s| {
            s.projects()
                .iter()
                .any(|p| p.main_folder.as_deref() == Some(folder))
        });
    if known {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::Denied,
            "a main folder must be a folder a session works in, or one chosen with Choose folder",
        ))
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl Core {
    /// True when `path` is exactly a saved project's main folder. The host uses this alongside
    /// [`Self::is_session_folder`] to admit "reveal in file manager" for a project without a
    /// session; a picked folder alone does not qualify until a project uses it.
    #[must_use]
    pub fn is_project_main_folder(&self, path: &str) -> bool {
        self.shared.projects.store.read(|store| {
            store
                .projects()
                .iter()
                .any(|project| project.main_folder.as_deref() == Some(path))
        })
    }

    /// `start`: the same gated `session/start` as `startSession`, inside the filing hint of `project`.
    async fn start_in_project(
        &self,
        origin: Origin,
        project: String,
        agent_config: serde_json::Value,
    ) -> Result<CommandResult, CoreError> {
        if self
            .shared
            .projects
            .store
            .read(|s| s.project(&project).is_none())
        {
            return Err(not_found(&project));
        }
        // only `worktrees start` sets `agentConfig.worktree` (the same rule as `startSession`).
        if agent_config.get("worktree").is_some_and(|w| !w.is_null()) {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "agentConfig.worktree is set by Vibe Work only: start a copy with the worktrees command",
            ));
        }
        // Boxed: the session call's future would otherwise be inlined into every command's future.
        Box::pin(crate::projects_filing::with_hint(
            Some(vw_projects::Hint::Project(project)),
            self.session_call(
                origin,
                "session/start",
                json!({"agentConfig": agent_config}),
            ),
        ))
        .await
    }

    /// Executes one project request for `origin`.
    pub(crate) async fn projects(
        &self,
        origin: Origin,
        request: ProjectsRequest,
    ) -> Result<CommandResult, CoreError> {
        let shared = &self.shared;
        if let ProjectsRequest::Create {
            main_folder: Some(folder),
            ..
        }
        | ProjectsRequest::SetFolder {
            folder: Some(folder),
            ..
        } = &request
        {
            check_folder(shared, folder)?;
        }
        if let ProjectsRequest::Move { session_ids, .. } = &request
            && (session_ids.is_empty() || session_ids.len() > MAX_MOVE)
        {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                format!("move 1 to {MAX_MOVE} sessions"),
            ));
        }
        let now = vw_projects::now_ms();
        let result = match request {
            ProjectsRequest::Start {
                project_id,
                agent_config,
            } => {
                return self
                    .start_in_project(origin, project_id, agent_config)
                    .await;
            }
            ProjectsRequest::List {} => {
                sweep_once(shared);
                list(shared)
            }
            ProjectsRequest::Refresh {} => {
                sweep(shared).await?;
                list(shared)
            }
            ProjectsRequest::Create { name, main_folder } => {
                let project = update(shared, move |s| {
                    s.create(&name, main_folder.as_deref(), now)
                })
                .await?;
                ProjectsResult::Created {
                    project: view(&project, 0),
                }
            }
            ProjectsRequest::Rename { project_id, name } => {
                update(shared, move |s| s.rename(&project_id, &name)).await?;
                ProjectsResult::Done {}
            }
            ProjectsRequest::Delete { project_id } => {
                let moved = update(shared, move |s| s.delete(&project_id)).await?;
                ProjectsResult::Moved {
                    moved: count(moved),
                }
            }
            ProjectsRequest::Move {
                session_ids,
                project_id,
            } => {
                // A session Vibe Work has loaded counts as known even before its first listing.
                let loaded: std::collections::BTreeSet<String> = session_ids
                    .iter()
                    .filter(|id| shared.store.view(id).is_some())
                    .cloned()
                    .collect();
                let moved = update(shared, move |s| {
                    s.move_sessions(&session_ids, project_id.as_deref(), |id| {
                        loaded.contains(id)
                    })
                })
                .await?;
                ProjectsResult::Moved {
                    moved: count(moved),
                }
            }
            ProjectsRequest::Place { project_id, before } => {
                update(shared, move |s| s.place(&project_id, before.as_deref())).await?;
                ProjectsResult::Done {}
            }
            ProjectsRequest::SetFolder { project_id, folder } => {
                update(shared, move |s| {
                    s.set_main_folder(&project_id, folder.as_deref())
                })
                .await?;
                ProjectsResult::Done {}
            }
        };
        Ok(CommandResult::Projects { result })
    }
}
