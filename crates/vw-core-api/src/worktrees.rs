//! The `worktrees` command: a session in its own copy of a git repository. This
//! module holds the shared state, the dispatcher and the read-only ops (`count`, `options`, `suggestName`,
//! `info`, `orphans`) plus `keep`; `start` is in `worktrees_start.rs`, `inspect` and `remove` in
//! `worktrees_remove.rs`.
//!
//! Depends on: vw-worktrees (git side, records), `core::Core`, `changes_facts` (a session's folder),
//! `worktrees_view`, `dispatch` (`forward`). Called by: `dispatch` (the `worktrees` command, the
//! `session/delete` hook, the refusal of a raw `agentConfig.worktree`).
//! Invariants:
//! - `suggestName` and `start` read or write only folders the user chose: picked in the native dialog
//!   in this run (`Gate::is_picked_folder`), or named by the Debug API (it acts as the user); a window
//!   `start` elsewhere needs the native dialog (`worktrees_start.rs`). `options` reads full repository facts only
//!   for those choices, but may identify the branch of an exact recorded Vibe Work copy without a pick.
//! - Records live in `<changes dir>/worktrees/records` (user-only); without a changes folder (tests, runs
//!   without app data) the command answers `internal` "worktrees are off".
//! - git work runs on `spawn_blocking`; starts and removals run one at a time (`WorktreesState::lock`).
//! - A client never sends `agentConfig.worktree` itself: only `start` adds it, after creating the branch
//!   (as the core adds `mcpServers`), so every copy goes through the preflight and gets a record.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{Value, json};
use vw_worktrees::{Decision, Records, WorktreeError};

use crate::changes_facts::facts;
use crate::command::{Command, Origin};
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::worktrees_view::{
    OrphanWorktree, WorktreeInfo, WorktreeOptions, WorktreeState, WorktreesRequest, WorktreesResult,
};

/// Why `options` / `suggestName` read nothing for a folder: it was not picked in the native
/// dialog in this run.
const NOT_PICKED: &str = "To work in a new copy, choose this folder with Choose folder.";

/// How long a requested copy counts as "being made" before a missing folder is reported as missing.
const PENDING_FOR_MS: u64 = 120_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionCopyState {
    Active,
    Archived,
    Deleted,
}

/// The worktree feature's state in the core.
pub(crate) struct WorktreesState {
    root: Option<PathBuf>,
    /// Vibe's worktree folder (`$VIBE_HOME/worktrees`): a copy is acted on only inside it.
    vibe_root: PathBuf,
    /// Cloned handles share failed-start records when durable writes are unavailable.
    records: OnceLock<Records>,
    /// One start or removal at a time (two starts could pick the same free name).
    pub(crate) lock: tokio::sync::Mutex<()>,
    /// First-turn setup facts already sent during this run; serialized across concurrent starts.
    pub(crate) setup_sent: tokio::sync::Mutex<HashSet<String>>,
}

impl WorktreesState {
    /// The state under `<changes_dir>/worktrees` (off without a changes folder); `vibe_home` is the
    /// server's `VIBE_HOME`.
    pub(crate) fn new(changes_dir: Option<&Path>, vibe_home: &Path) -> Self {
        Self {
            root: changes_dir.map(|d| d.join("worktrees")),
            vibe_root: vibe_home.join("worktrees"),
            records: OnceLock::new(),
            lock: tokio::sync::Mutex::new(()),
            setup_sent: tokio::sync::Mutex::new(HashSet::new()),
        }
    }

    fn root(&self) -> Result<&Path, CoreError> {
        self.root.as_deref().ok_or_else(|| {
            CoreError::new(
                ErrorCode::Internal,
                "worktrees are off in this run (no app data folder)",
            )
        })
    }

    /// [`WorktreesState::new`] from the core's config and Vibe's home (the instruction paths carry it).
    pub(crate) fn of(
        config: &crate::core::CoreConfig,
        paths: &crate::instructions::InstructionsPaths,
    ) -> Self {
        Self::new(config.changes_dir.as_deref(), &paths.own_home)
    }

    /// Vibe's worktree folder.
    pub(crate) fn vibe_root(&self) -> &Path {
        &self.vibe_root
    }

    /// The record folder.
    pub(crate) fn records(&self) -> Result<Records, CoreError> {
        if let Some(records) = self.records.get() {
            return Ok(records.clone());
        }
        let records = Records::open(&self.root()?.join("records")).map_err(core_error)?;
        Ok(self.records.get_or_init(|| records).clone())
    }

    /// The inspection's scratch folder.
    pub(crate) fn scan_dir(&self) -> Result<PathBuf, CoreError> {
        Ok(self.root()?.join("scan"))
    }
}

/// Maps a vw-worktrees error to the core's codes.
pub(crate) fn core_error(error: WorktreeError) -> CoreError {
    match error {
        WorktreeError::NotGit(m) | WorktreeError::InvalidName(m) | WorktreeError::Taken(m) => {
            CoreError::new(ErrorCode::InvalidParams, m)
        }
        WorktreeError::NotOffered(m) => CoreError::new(ErrorCode::Denied, m),
        WorktreeError::Stale(m) => CoreError::new(ErrorCode::Stale, m),
        other => CoreError::new(ErrorCode::Internal, other.to_string()),
    }
}

/// Runs git work off the async runtime.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("worktree task failed: {e}")))
}

/// The refusal of a client's own `agentConfig.worktree` (start, continue, fork, resume, also through a raw
/// `call`): only the `worktrees start` op may add it.
pub(crate) fn raw_worktree_refusal(command: &Command) -> Option<CoreError> {
    // A client's own removal (anything but a side effect free probe): only `worktrees remove` removes, for a
    // copy Vibe Work made, after the inspection.
    if let Command::Call { method, params, .. } = command
        && method == "workspace/git/worktrees/remove"
        && !(params.get("inspect").and_then(Value::as_bool) == Some(true)
            && params.get("force").and_then(Value::as_bool) != Some(true))
    {
        return Some(CoreError::new(
            ErrorCode::Denied,
            "a worktree is removed only through the worktrees command (op remove), after its inspection",
        ));
    }
    let config = match command {
        Command::StartSession { agent_config }
        | Command::ResumeSession { agent_config, .. }
        | Command::ForkSession { agent_config, .. } => agent_config.as_ref(),
        Command::Call { method, params, .. }
            if matches!(
                method.as_str(),
                "session/start" | "session/continue" | "session/fork" | "session/resume"
            ) =>
        {
            params.get("agentConfig")
        }
        _ => None,
    }?;
    config.get("worktree").filter(|w| !w.is_null())?;
    Some(CoreError::new(
        ErrorCode::InvalidParams,
        "agentConfig.worktree is set by Vibe Work only: start a session in its own copy with the worktrees command (op start)",
    ))
}

/// The `session/delete` hook: the session's copy may now be an orphan.
pub(crate) fn session_deleted(shared: &Shared, session_id: &str) {
    if let Ok(records) = shared.worktrees.records()
        && let Err(error) = records.update(session_id, |r| r.session_deleted = true)
    {
        tracing::warn!(session_id, %error, "worktrees: could not mark a deleted session");
    }
}

/// The folder of the worktree on `branch` in the repository at `repo_root` (git's list).
pub(crate) fn find_path(repo_root: &Path, branch: &str) -> Option<PathBuf> {
    vw_worktrees::repo_info::worktrees(repo_root)
        .ok()?
        .into_iter()
        .skip(1)
        .find(|w| w.branch.as_deref() == Some(branch))
        .map(|w| w.path)
}

/// Recognize only a recorded copy's exact, existing folder beneath Vibe's own worktree root.
/// This is a read-only label for the new-chat panel, never a native-pick authorization.
fn managed_copy_branch(records: &Records, vibe_root: &Path, folder: &Path) -> Option<String> {
    let folder = folder.canonicalize().ok()?;
    let vibe_root = vibe_root.canonicalize().ok()?;
    if !folder.starts_with(&vibe_root) {
        return None;
    }
    records.all().into_iter().find_map(|record| {
        if record.decision == Some(Decision::Removed) {
            return None;
        }
        let path = find_path(&record.repo_root, &record.branch)?;
        (path.file_name()? == record.folder.as_str() && path.canonicalize().ok()? == folder)
            .then_some(record.branch)
    })
}

impl Core {
    /// Executes one worktree request for `origin`.
    ///
    /// # Errors
    /// `invalidParams` (not git, a bad or taken name, a raw `worktree`), `denied` (not offered, a removal that
    /// cannot happen), `stale` (the copy changed since the user looked), `confirmationDeclined`, `notFound`,
    /// `internal` (git failures, feature off).
    pub async fn worktrees(
        &self,
        origin: Origin,
        request: WorktreesRequest,
    ) -> Result<WorktreesResult, CoreError> {
        match request {
            WorktreesRequest::Count {} => self.worktree_count().await,
            WorktreesRequest::Options { cwd } => self.worktree_options(origin, cwd).await,
            WorktreesRequest::SuggestName { cwd, text } => {
                self.worktree_suggest(origin, cwd, text).await
            }
            WorktreesRequest::Start {
                agent_config,
                base,
                branch,
            } => {
                self.worktree_start(origin, agent_config, base, branch)
                    .await
            }
            WorktreesRequest::Info { session_id } => Ok(WorktreesResult::Info {
                info: self.worktree_info(&session_id).await?,
            }),
            WorktreesRequest::Inspect { session_id } => Ok(WorktreesResult::Inspection {
                inspection: self.worktree_inspect(origin, &session_id).await?,
            }),
            WorktreesRequest::Remove {
                session_id,
                fingerprint,
            } => Ok(WorktreesResult::Removed {
                outcome: self
                    .worktree_remove(origin, &session_id, &fingerprint)
                    .await?,
            }),
            WorktreesRequest::Keep { session_id } => self.worktree_keep(&session_id),
            WorktreesRequest::Orphans {} => self.worktree_orphans().await,
        }
    }

    /// Whether `cwd` counts as the user's own choice for `origin`: picked in the native dialog
    /// in this run, or named by the Debug API, which acts as the user (its `pickFolder` is the picker).
    pub(crate) fn chosen_by_user(&self, origin: Origin, cwd: &Path) -> bool {
        origin == Origin::DebugApi || self.shared.gate.is_picked_folder(cwd)
    }

    /// `options`: full repository facts only for a folder the user chose (`chosen_by_user`). An exact
    /// recorded copy may expose its branch without a pick; other unpicked folders are not read.
    async fn worktree_options(
        &self,
        origin: Origin,
        cwd: String,
    ) -> Result<WorktreesResult, CoreError> {
        if !self.chosen_by_user(origin, Path::new(&cwd)) {
            // A saved Vibe Work copy may be selected from the copies list without a native picker.
            // Recognize only its exact recorded folder; this does not grant start/suggest authority.
            let records = self.shared.worktrees.records().ok();
            let vibe_root = self.shared.worktrees.vibe_root().to_path_buf();
            let folder = PathBuf::from(&cwd);
            let existing_branch = blocking(move || {
                records.and_then(|r| managed_copy_branch(&r, &vibe_root, &folder))
            })
            .await?;
            return Ok(WorktreesResult::Options {
                options: WorktreeOptions {
                    picked: false,
                    is_git: existing_branch.is_some(),
                    existing_branch,
                    repo_root: None,
                    head: None,
                    branches: Vec::new(),
                    offered: false,
                    reason: Some(NOT_PICKED.to_owned()),
                    suggestion: None,
                    include: None,
                },
            });
        }
        // Answers without the records too (worktrees off): then nothing can be offered.
        let records = self.shared.worktrees.records().ok();
        let vibe_root = self.shared.worktrees.vibe_root().to_path_buf();
        let options = blocking(move || -> Result<WorktreeOptions, CoreError> {
            let Some(info) = vw_worktrees::read(Path::new(&cwd)).map_err(core_error)? else {
                return Ok(WorktreeOptions {
                    picked: true,
                    is_git: false,
                    existing_branch: None,
                    repo_root: None,
                    head: None,
                    branches: Vec::new(),
                    offered: false,
                    reason: None,
                    suggestion: None,
                    include: None,
                });
            };
            let suggestion = records
                .as_ref()
                .and_then(|r| free_name(&info, r, ""))
                .map(|s| s.branch);
            let existing_branch = records
                .as_ref()
                .and_then(|r| managed_copy_branch(r, &vibe_root, &info.root));
            // The preview of the `.worktreeinclude` copy, only for a repository a copy is offered for
            // (the preflight ran: no repository program is named in its config).
            let include = (info.not_offered.is_none() && records.is_some()).then(|| {
                let others: Vec<PathBuf> = info
                    .worktrees
                    .iter()
                    .skip(1)
                    .map(|w| w.path.clone())
                    .collect();
                vw_worktrees::include::plan(info.main_root(), &others)
            });
            Ok(WorktreeOptions {
                picked: true,
                is_git: true,
                existing_branch,
                repo_root: Some(info.main_root().display().to_string()),
                offered: info.not_offered.is_none() && records.is_some(),
                reason: info.not_offered.clone().or_else(|| {
                    records
                        .is_none()
                        .then(|| "Copies are off in this run (no app data folder).".to_owned())
                }),
                head: Some(info.head),
                branches: info.branches,
                suggestion,
                include,
            })
        })
        .await??;
        Ok(WorktreesResult::Options { options })
    }

    async fn worktree_suggest(
        &self,
        origin: Origin,
        cwd: String,
        text: String,
    ) -> Result<WorktreesResult, CoreError> {
        // Only a folder the user chose is read (the suggestion lists the repository's branch names).
        if !self.chosen_by_user(origin, Path::new(&cwd)) {
            return Err(CoreError::new(ErrorCode::Denied, NOT_PICKED));
        }
        let records = self.shared.worktrees.records()?;
        let pick = blocking(move || -> Result<vw_worktrees::Suggestion, CoreError> {
            let info = vw_worktrees::read(Path::new(&cwd))
                .map_err(core_error)?
                .ok_or_else(|| {
                    CoreError::new(
                        ErrorCode::InvalidParams,
                        "the folder is not in a git repository",
                    )
                })?;
            free_name(&info, &records, &text).ok_or_else(|| {
                CoreError::new(
                    ErrorCode::InvalidParams,
                    "no free branch name found; type one",
                )
            })
        })
        .await??;
        Ok(WorktreesResult::Name {
            branch: pick.branch,
            folder: pick.folder,
        })
    }

    /// The session's copy for the header: from the record, else from the session's folder when it is a
    /// linked worktree (a copy Vibe Work did not make, e.g. a fork's).
    pub(crate) async fn worktree_info(
        &self,
        session_id: &str,
    ) -> Result<Option<WorktreeInfo>, CoreError> {
        let records = self.shared.worktrees.records()?;
        let cwd = self
            .shared
            .store
            .view(session_id)
            .and_then(|v| facts(&v.state).cwd);
        let id = session_id.to_owned();
        blocking(move || info_of(&records, &id, cwd.as_deref())).await
    }

    fn worktree_keep(&self, session_id: &str) -> Result<WorktreesResult, CoreError> {
        let records = self.shared.worktrees.records()?;
        records
            .update(session_id, |r| r.decision = Some(Decision::Kept))
            .map_err(core_error)?;
        Ok(WorktreesResult::Kept {})
    }

    async fn worktree_count(&self) -> Result<WorktreesResult, CoreError> {
        let records = self.shared.worktrees.records()?;
        let vibe_root = self.shared.worktrees.vibe_root().to_path_buf();
        let count = blocking(move || count_copies(&records, &vibe_root)).await?;
        Ok(WorktreesResult::Count { count })
    }

    async fn worktree_orphans(&self) -> Result<WorktreesResult, CoreError> {
        let records = self.shared.worktrees.records()?;
        let mut orphans = Vec::new();
        // Loaded and list-only sessions can both use a copy. Their cached archive state or folder may lag a
        // completed archive/move, so read the server only for IDs whose remembered folder matches this copy.
        let known_folders: Vec<(String, PathBuf)> = self
            .shared
            .names
            .session_folders(&self.shared.store)
            .into_iter()
            .filter_map(|(id, cwd)| Path::new(&cwd).canonicalize().ok().map(|path| (id, path)))
            .collect();
        for record in records.all() {
            if record.decision == Some(Decision::Removed) {
                continue;
            }
            let kept = record.decision == Some(Decision::Kept);
            let recovery = records.is_recovery(&record.session_id);
            // A session archived before its first turn is recorded on its copy (Vibe never saved it, so
            // its own archive state cannot be read back); such a copy is listed like a deleted session's.
            let state = if record.session_deleted || record.session_archived {
                SessionCopyState::Deleted
            } else if kept || self.shared.store.view(&record.session_id).is_none() {
                self.session_copy_state(&record.session_id).await.0
            } else {
                SessionCopyState::Active
            };
            if !recovery
                && state != SessionCopyState::Deleted
                && !(kept && state == SessionCopyState::Archived)
            {
                continue;
            }
            let (root, branch) = (record.repo_root.clone(), record.branch.clone());
            let Some(path) = blocking(move || find_path(&root, &branch)).await? else {
                continue;
            };
            if !path.is_dir() {
                continue;
            }
            let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            let mut in_use = false;
            for (id, _) in known_folders
                .iter()
                .filter(|(_, folder)| folder.starts_with(&canonical_path))
            {
                // A failed start is reviewable even when its safe archive failed. Its own session
                // remains a removal blocker, while another active owner still hides the copy.
                if recovery && id == &record.session_id {
                    continue;
                }
                let (state, cwd) = self.session_copy_state(id).await;
                if state == SessionCopyState::Active
                    && cwd
                        .and_then(|path| path.canonicalize().ok())
                        .is_none_or(|folder| folder.starts_with(&canonical_path))
                {
                    in_use = true;
                    break;
                }
            }
            if in_use {
                continue;
            }
            orphans.push(OrphanWorktree {
                session_id: record.session_id,
                kept,
                path: Some(path.display().to_string()),
                branch: record.branch,
                base: record.base_name,
                repo_root: Some(record.repo_root.display().to_string()),
                created_at_ms: record.created_at_ms,
            });
        }
        Ok(WorktreesResult::Orphans { orphans })
    }

    /// Read the original session when the store cannot establish that a copy is still active.
    /// A failed read leaves it active, so an uncertain copy does not enter the inventory.
    async fn session_copy_state(&self, session_id: &str) -> (SessionCopyState, Option<PathBuf>) {
        // Vibe keeps no archive state for a session archived before its first turn; its copy record does.
        let recorded_archive = self
            .shared
            .worktrees
            .records()
            .ok()
            .and_then(|records| records.get(session_id))
            .is_some_and(|record| record.session_archived);
        let read = self
            .shared
            .gate
            .call("session/read", json!({"sessionId": session_id}), None)
            .await;
        match read {
            Ok(reply) => {
                let cwd = reply
                    .result
                    .pointer("/state/session/cwd")
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                let state = if recorded_archive
                    || reply
                        .result
                        .pointer("/state/session/archivedAt")
                        .is_some_and(|value| !value.is_null())
                {
                    SessionCopyState::Archived
                } else {
                    SessionCopyState::Active
                };
                (state, cwd)
            }
            Err(error) => {
                let error = CoreError::from_gate("session/read", &error);
                if error.server_code.as_deref() == Some("not_found") {
                    (SessionCopyState::Deleted, None)
                } else {
                    (SessionCopyState::Active, None)
                }
            }
        }
    }

    /// Shows the native dialog with `summary` for `requester` (the dialog leads with who asks).
    /// Declined = `confirmationDeclined`.
    pub(crate) async fn ask_native(
        &self,
        requester: vw_gate::Requester,
        method: &str,
        params: Value,
        summary: String,
    ) -> Result<(), CoreError> {
        use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};
        let summary = if let Some(id) = params.get("sessionId").and_then(Value::as_str) {
            let title = self.shared.names.title_of(&self.shared.store, id);
            with_session(&summary, id, title.as_deref())
        } else {
            summary
        };
        let request = ConfirmationRequest {
            method: method.to_owned(),
            requester,
            summary,
            params,
            always: None,
        };
        match self.shared.confirmer.confirm(&request).await {
            Confirmation::Approved | Confirmation::ApprovedAlways => Ok(()),
            Confirmation::Declined => Err(CoreError {
                method: Some(method.to_owned()),
                ..CoreError::new(
                    ErrorCode::ConfirmationDeclined,
                    "the change was not confirmed in the native dialog",
                )
            }),
        }
    }
}

fn count_copies(records: &Records, vibe_root: &Path) -> usize {
    let mut paths = HashSet::new();
    for record in records.all() {
        if record.decision == Some(Decision::Removed) {
            continue;
        }
        let Some(path) = find_path(&record.repo_root, &record.branch) else {
            continue;
        };
        if path.is_dir() && crate::worktrees_remove::placement(&path, vibe_root).is_none() {
            paths.insert(path);
        }
    }
    paths.len()
}

fn with_session(summary: &str, id: &str, title: Option<&str>) -> String {
    let session = title.map_or_else(
        || format!("Session {}", vw_gate::dialog_quote(id)),
        |title| {
            format!(
                "Session {} (ID {})",
                vw_gate::dialog_quote(title),
                vw_gate::dialog_quote(id)
            )
        },
    );
    format!("{session}\n\n{summary}")
}

/// A free `vibe/<slug>` for `text` in the repository: no branch and no worktree folder (git's list, and the
/// folders of our records) by that name.
pub(crate) fn free_name(
    info: &vw_worktrees::RepoInfo,
    records: &Records,
    text: &str,
) -> Option<vw_worktrees::Suggestion> {
    let folders: Vec<String> = info
        .worktrees
        .iter()
        .filter_map(|w| w.path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .chain(records.all().into_iter().map(|r| r.folder))
        .collect();
    vw_worktrees::suggest(text, |branch, folder| {
        info.branches.iter().any(|b| b.name == branch) || folders.iter().any(|f| f == folder)
    })
}

fn info_of(records: &Records, session_id: &str, cwd: Option<&Path>) -> Option<WorktreeInfo> {
    if let Some(record) = records.get(session_id) {
        let path = find_path(&record.repo_root, &record.branch);
        let state = if record.decision == Some(Decision::Removed) {
            WorktreeState::Removed
        } else if path.as_ref().is_some_and(|p| p.is_dir()) {
            WorktreeState::Ready
        } else if vw_changes::now_ms().saturating_sub(record.created_at_ms) < PENDING_FOR_MS {
            WorktreeState::Pending
        } else {
            WorktreeState::Missing
        };
        return Some(WorktreeInfo {
            session_id: session_id.to_owned(),
            branch: record.branch,
            base: Some(record.base_name),
            path: path.map(|p| p.display().to_string()),
            state,
            made_here: true,
            decision: record.decision,
        });
    }
    // A session sitting in a linked worktree Vibe Work did not make (a fork of a copy, a CLI session).
    let Ok(Some(info)) = vw_worktrees::read(cwd?) else {
        return None;
    };
    if info.worktrees.len() < 2 || info.main_root() == info.root {
        return None;
    }
    let vw_worktrees::Head::Branch { name } = info.head else {
        return None;
    };
    Some(WorktreeInfo {
        session_id: session_id.to_owned(),
        branch: name,
        base: None,
        path: Some(info.root.display().to_string()),
        state: WorktreeState::Ready,
        made_here: false,
        decision: None,
    })
}

#[cfg(test)]
#[path = "worktrees_tests.rs"]
mod kg1_tests;
