//! `worktrees start`: a new session in its own copy of the folder's repository, on a new branch
//! started from the base the user picked.
//!
//! Steps: read the repository and run the preflight (refused: not git, no commit yet, a program the
//! repository's own config names that git would run while the copy is made); resolve the base;
//! check the branch name and that neither the branch nor the folder name is taken; ask the native dialog when
//! the window starts in a folder that was not picked (below); create the branch at the base (create-only,
//! hardened runner); `session/start` with `agentConfig.worktree = {kind: "create", branch, name}` (Vibe
//! checks the existing branch out and never fetches); write the record. A failed record retains recovery
//! metadata and archives the session if possible, keeping its copy and branch. A failed start deletes the branch
//! again (only while it is still at the base). Once the copy exists, inspect package file names at its root
//! and return a one-line setup hint; the first turn gets a fresh hint through [`Core::first_turn_setup`].
//!
//! Depends on: vw-worktrees, `worktrees` (state, helpers), `dispatch` (`session_call`), `command`. Called by:
//! `worktrees`.
//! Invariants:
//! - The user's index, `HEAD`, stash and config are never written; the only new ref is the branch.
//! - For folder selection, the Debug API acts as the user, so a folder it names is
//!   registered as picked and the copy starts without a dialog. The window starts without the native dialog
//!   in a folder the user picked natively in this run (`Gate::is_picked_folder`, a folder inside one counts);
//!   any other folder asks the native dialog first, naming the repository, the branch and the base and saying
//!   the folder was not chosen with Choose folder. The window's folder rules guard against its webview;
//!   `ui/worktrees folder` picks the folder through `pickFolder`.
//! - A folder that was not picked is never read before the user's consent. The checks'
//!   distinct answers, and "dialog or refusal" itself, told a caller facts about any path). The dialog comes
//!   first, naming the folder as sent; after it any failed check gives one fixed refusal, and so does a folder
//!   that is not exactly its repository's main folder (a symlink, a subfolder or a linked worktree would put
//!   the branch somewhere the dialog did not name).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use vw_worktrees::{IncludeReport, Record, branch, include, naming};

use crate::command::{CommandResult, Origin, requester};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::worktrees::{blocking, core_error};
use crate::worktrees_view::{WorktreeState, WorktreesResult};

/// The one refusal of a start in a folder that was not picked, whatever the checks found (module docs).
const NOT_PICKED_REFUSAL: &str =
    "No copy can be started in this folder. Choose it with Choose folder to see why.";

/// The folder of a start's `agentConfig` (`cwd`); refuses a config that is not an object, has no `cwd`, or
/// carries `worktree` itself (only the start adds it).
fn start_folder(agent_config: &Value) -> Result<PathBuf, CoreError> {
    let Some(config) = agent_config.as_object() else {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "agentConfig must be an object",
        ));
    };
    if config.get("worktree").is_some_and(|w| !w.is_null()) {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "agentConfig may not carry worktree: the start adds it",
        ));
    }
    config
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| CoreError::new(ErrorCode::InvalidParams, "agentConfig.cwd names the folder"))
}

/// How long the copy of `.worktreeinclude` files waits for Vibe to make the folder (under 10 s).
const FOLDER_WAIT: Duration = Duration::from_secs(20);
/// How often it looks.
const FOLDER_POLL: Duration = Duration::from_millis(100);

/// A short install/check prerequisite from regular package files at the copy root. Reading names only keeps
/// repository content from becoming instructions; the agent decides whether and when to run these commands.
fn setup_line(root: &Path) -> Option<String> {
    let has = |name: &str| {
        std::fs::symlink_metadata(root.join(name)).is_ok_and(|meta| meta.file_type().is_file())
    };
    let mut steps = Vec::new();
    if has("pnpm-lock.yaml") {
        steps.push("pnpm install --frozen-lockfile");
    } else if has("package-lock.json") {
        steps.push("npm ci");
    } else if has("yarn.lock") {
        steps.push("yarn install");
    }
    if has("Cargo.toml") {
        steps.push("cargo fetch");
    }
    if has("uv.lock") {
        steps.push("uv sync");
    } else if has("requirements.txt") {
        steps.push("python -m pip install -r requirements.txt");
    } else if has("pyproject.toml") {
        steps.push("python -m pip install -e .");
    }
    if has("go.mod") {
        steps.push("go mod download");
    }
    (!steps.is_empty()).then(|| format!("Copy setup before checks: {}", steps.join("; ")))
}

/// What the checks before the start found.
struct Planned {
    main_root: PathBuf,
    base_commit: String,
    folder: String,
}

fn copy_record(session_id: String, planned: Planned, base: String, branch: String) -> Record {
    Record {
        session_id,
        repo_root: planned.main_root,
        base_name: base,
        base_commit: planned.base_commit,
        branch,
        folder: planned.folder,
        created_branch: true,
        created_at_ms: vw_changes::now_ms(),
        decision: None,
        session_deleted: false,
        session_archived: false,
    }
}

/// The checks before anything is written (blocking).
fn plan(
    cwd: &Path,
    base: &str,
    new_branch: &str,
    taken_folders: &[String],
) -> Result<Planned, CoreError> {
    let info = vw_worktrees::read(cwd)
        .map_err(core_error)?
        .ok_or_else(|| {
            CoreError::new(
                ErrorCode::InvalidParams,
                "the folder is not in a git repository",
            )
        })?;
    if let Some(reason) = &info.not_offered {
        return Err(CoreError::new(ErrorCode::Denied, reason.clone()));
    }
    let base_commit =
        vw_worktrees::repo_info::resolve_base(&info.root, base).map_err(core_error)?;
    branch::validate(&info.root, new_branch).map_err(core_error)?;
    if info.branches.iter().any(|b| b.name == new_branch) {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            format!("the branch {new_branch:?} exists already; pick another name"),
        ));
    }
    let folder = naming::folder_for(new_branch);
    let in_use = info
        .worktrees
        .iter()
        .filter_map(|w| w.path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .chain(taken_folders.iter().cloned())
        .any(|f| f == folder);
    if in_use {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            format!("a copy folder named {folder:?} exists already; pick another branch name"),
        ));
    }
    Ok(Planned {
        main_root: info.main_root().to_path_buf(),
        base_commit,
        folder,
    })
}

impl Core {
    /// The package line for an untouched session copy's first turn. `Some(None)` means it is the first turn
    /// but no package file was found; after a successful turn either case is remembered for this run.
    pub(crate) async fn first_turn_setup(&self, session_id: &str) -> Option<Option<String>> {
        let record = self.shared.worktrees.records().ok()?.get(session_id)?;
        if record.decision == Some(vw_worktrees::Decision::Removed) {
            return None;
        }
        let view = self.shared.store.view(session_id)?;
        if !view.state.pointer("/turns")?.as_array()?.is_empty() {
            return None;
        }
        let info = self.worktree_info(session_id).await.ok()??;
        let path = info.path.filter(|_| info.state == WorktreeState::Ready)?;
        Some(setup_line(Path::new(&path)))
    }

    /// Waits for the new copy before reading its root or applying included files.
    async fn wait_for_copy(&self, session_id: &str) -> Option<PathBuf> {
        let waited = tokio::time::Instant::now();
        loop {
            if let Ok(Some(info)) = self.worktree_info(session_id).await
                && info.state == WorktreeState::Ready
                && let Some(path) = info.path
            {
                return Some(PathBuf::from(path));
            }
            if waited.elapsed() > FOLDER_WAIT {
                return None;
            }
            tokio::time::sleep(FOLDER_POLL).await;
        }
    }

    /// A session already exists: retain ownership, attempt a non-destructive archive and report its
    /// real ID/path. A record error never grants permission to delete the user's copy or branch.
    async fn recover_failed_copy(
        &self,
        origin: Origin,
        records: &vw_worktrees::Records,
        record: &Record,
        cause: vw_worktrees::WorktreeError,
    ) -> CoreError {
        let _ = records.recover(record);
        let path = self.wait_for_copy(&record.session_id).await;
        let archive = self
            .session_call(
                origin,
                "session/archive",
                json!({
                    "sessionId": record.session_id, "archived": true
                }),
            )
            .await;
        // The archive updates recovery metadata as well. Check that its latest state is persisted,
        // so a second write failure is never presented as durable recovery.
        let durable = records
            .get(&record.session_id)
            .is_some_and(|current| records.recover(&current).is_ok());
        let location = path.as_ref().map_or_else(
            || {
                format!(
                    "pending copy on branch {:?} in {}",
                    record.branch,
                    record.repo_root.display()
                )
            },
            |path| path.display().to_string(),
        );
        self.shared
            .emit(crate::event::CoreEvent::WorktreesChanged {});
        CoreError::new(
            ErrorCode::Internal,
            format!(
                "The copy record could not be saved: {cause}. Session {} and copy {} were kept; review it in Leftover copies. {}{}",
                record.session_id,
                location,
                if durable {
                    "Recovery metadata was saved separately."
                } else {
                    "Recovery metadata is available only for this run; keep the session ID and copy path before quitting."
                },
                if archive.is_err() {
                    " The archive did not finish; Review checks whether the session still uses the copy."
                } else {
                    ""
                },
            ),
        )
    }

    /// Copies the `.worktreeinclude` files of `main_root` into the new copy's folder. The folder is
    /// the one git lists for our branch (never a path from the repository).
    /// Every problem is in the returned report; nothing here fails the start.
    async fn copy_includes(main_root: &Path, dest: Option<&Path>) -> IncludeReport {
        if std::fs::symlink_metadata(main_root.join(include::INCLUDE_FILE)).is_err() {
            return IncludeReport::default();
        }
        let Some(dest) = dest else {
            return IncludeReport {
                found: true,
                stopped: Some(
                    "The copy's folder did not appear in time, so .worktreeinclude was not applied."
                        .to_owned(),
                ),
                ..IncludeReport::default()
            };
        };
        let root = main_root.to_path_buf();
        let dest = dest.to_path_buf();
        blocking(move || {
            let others: Vec<PathBuf> = vw_worktrees::repo_info::worktrees(&root)
                .map(|list| list.into_iter().skip(1).map(|w| w.path).collect())
                .unwrap_or_default();
            let plan = include::plan(&root, &others);
            include::apply(&root, &dest, &plan)
        })
        .await
        .unwrap_or_else(|error| IncludeReport {
            found: true,
            stopped: Some(format!(
                "The files from .worktreeinclude were not copied: {}",
                error.message
            )),
            ..IncludeReport::default()
        })
    }

    /// The native dialog before a window start in a folder that was not picked (module docs). The text names
    /// `folder` as the caller sent it (asked before anything reads it), the branch and the base, every value
    /// through `vw_gate::dialog_value`. Declined = `confirmationDeclined`. The Debug API never gets here: its
    /// folder is registered as picked first.
    async fn ask_start(
        &self,
        origin: Origin,
        folder: &Path,
        new_branch: &str,
        base: &str,
    ) -> Result<(), CoreError> {
        let summary = format!(
            "Start a session in a new copy of {} on a new branch {} from {}? This folder was not chosen with \
             Choose folder since the app started.",
            vw_gate::dialog_quote(&folder.display().to_string()),
            vw_gate::dialog_quote(new_branch),
            vw_gate::dialog_quote(base),
        );
        self.ask_native(
            requester(origin),
            "worktrees/start",
            json!({"cwd": folder.display().to_string(), "branch": new_branch, "base": base}),
            summary,
        )
        .await
    }

    /// `worktrees start`. Returns the new session and its copy.
    ///
    /// # Errors
    /// See the module docs; `denied` with one fixed message for a folder that was not picked and cannot get a
    /// copy; `confirmationDeclined` from the native dialog; the start's own errors.
    #[allow(clippy::too_many_lines)] // one ordered start transaction, including the count refresh event
    pub(crate) async fn worktree_start(
        &self,
        origin: Origin,
        agent_config: Value,
        base: String,
        new_branch: String,
    ) -> Result<WorktreesResult, CoreError> {
        let cwd = start_folder(&agent_config)?;
        if origin == Origin::DebugApi {
            // The Debug API acts as the user, so the folder it names is the user's choice, like a
            // native pick (its `pickFolder`); the session start that follows may then trust it.
            self.shared
                .gate
                .register_picked_folder(&cwd)
                .map_err(|e| CoreError::from_gate("worktrees/start", &e))?;
        }
        let picked = self.shared.gate.is_picked_folder(&cwd);
        // A folder that was not picked: the user's consent comes before anything reads it (finding 4 of
        // The dialog names the folder as the caller sent it.
        if !picked {
            self.ask_start(origin, &cwd, &new_branch, &base).await?;
        }
        let _one_at_a_time = self.shared.worktrees.lock.lock().await;
        let records = self.shared.worktrees.records()?;
        let taken: Vec<String> = records.all().into_iter().map(|r| r.folder).collect();
        let (c, b, n) = (cwd.clone(), base.clone(), new_branch.clone());
        let planned = match blocking(move || plan(&c, &b, &n, &taken)).await? {
            // The approved folder must be the repository's main folder exactly as named in the dialog (no
            // symlink, subfolder or linked worktree that would put the branch somewhere the dialog did not
            // name).
            Ok(planned) if !picked && planned.main_root != cwd => {
                return Err(CoreError::new(ErrorCode::Denied, NOT_PICKED_REFUSAL));
            }
            Ok(planned) => planned,
            Err(_) if !picked => {
                return Err(CoreError::new(ErrorCode::Denied, NOT_PICKED_REFUSAL));
            }
            Err(error) => return Err(error),
        };
        let (root, b, commit) = (
            planned.main_root.clone(),
            new_branch.clone(),
            planned.base_commit.clone(),
        );
        blocking(move || {
            // The preflight again, right before the branch: an agent could have added a setting meanwhile.
            if let Some(reason) = vw_worktrees::preflight(&root).map_err(core_error)? {
                return Err(CoreError::new(ErrorCode::Denied, reason));
            }
            branch::create(&root, &b, &commit).map_err(core_error)
        })
        .await??;
        let mut start = agent_config.clone();
        if let Some(object) = start.as_object_mut() {
            object.insert(
                "worktree".to_owned(),
                json!({"kind": "create", "branch": new_branch, "name": planned.folder}),
            );
        }
        let result = self
            .session_call(origin, "session/start", json!({"agentConfig": start}))
            .await;
        let snapshot = match result {
            Ok(CommandResult::Session { snapshot, .. }) => snapshot,
            other => {
                let (root, b, commit) = (
                    planned.main_root.clone(),
                    new_branch.clone(),
                    planned.base_commit.clone(),
                );
                let _ = blocking(move || branch::delete_if_at(&root, &b, &commit)).await;
                return Err(match other {
                    Err(error) => error,
                    Ok(_) => CoreError::bad_result("session/start", "state.session.id"),
                });
            }
        };
        let main_root = planned.main_root.clone();
        let record = copy_record(snapshot.session_id.clone(), planned, base, new_branch);
        if let Err(error) = records.put(&record) {
            return Err(self
                .recover_failed_copy(origin, &records, &record, error)
                .await);
        }
        // Copy the `.worktreeinclude` files into the new folder before the window sends the first
        // message (waits for Vibe to make the folder). A failure never fails the start (it is in the report).
        let copy_path = self.wait_for_copy(&snapshot.session_id).await;
        crate::entry_projects::finish_copy(
            &self.shared,
            &snapshot.session_id,
            copy_path.as_deref(),
        )
        .await;
        let include = Self::copy_includes(&main_root, copy_path.as_deref()).await;
        // After the copy, so `info` already shows the folder when Vibe made it during the wait.
        let info = self
            .worktree_info(&snapshot.session_id)
            .await?
            .ok_or_else(|| {
                CoreError::new(ErrorCode::Internal, "the copy's record was not readable")
            })?;
        let setup = copy_path.as_deref().and_then(setup_line);
        self.shared
            .emit(crate::event::CoreEvent::WorktreesChanged {});
        Ok(WorktreesResult::Started {
            snapshot,
            info,
            include,
            setup,
        })
    }
}
