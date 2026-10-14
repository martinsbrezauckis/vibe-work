//! The `git` command: the shared state, the dispatcher,
//! the branch bar (`status`), the Commit dialog (`prepareCommit`, `draftMessage`, `commit`) and the per-project
//! settings. Push, pull requests, the PR card and "Bring into main" are in `git_publish.rs`.
//!
//! Depends on: vw-git (git and forge side), `changes_facts` (the session's folder and running turn), `worktrees`
//! (records: is this session a copy, its base), `changes` (committed files leave the review), the key service
//! (the message model). Called by: `dispatch` (the `git` command).
//! Invariants:
//! - Without a changes folder (tests, runs without app data) the command answers `internal` "git is off".
//! - git work runs on `spawn_blocking`; a commit, push or bring-in of one repository runs one at a time
//!   (`GitHub::lock`).
//! - Nothing here reaches the network except the message model call (the user's key, the core's own direct
//!   call like OCR) and what `git_publish` does after a confirmation.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use vw_git::GitError;
use vw_git::commit::{self, Expected};
use vw_git::pr::PrCard;
use vw_git::state::{self, BranchState};

use crate::changes_facts::facts;
use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::git_view::{
    CommitPlanView, GitLocation, GitRequest, GitResult, GitSettings, GitStatusView,
};
use crate::worktrees::blocking;

/// The model that drafts commit messages unless the config names another: Mistral Large 4.
/// The pinned `mistral-large-4` id avoids changes to a floating `latest` alias.
pub const DEFAULT_COMMIT_MODEL: &str = "mistral-large-4";

/// The cached PR card of a session, and when the forge was last asked.
#[derive(Debug, Clone, Default)]
pub(crate) struct CardCache {
    pub(crate) card: Option<PrCard>,
    pub(crate) source: Option<crate::git_view::PrSource>,
    pub(crate) fetched_at_ms: u64,
    pub(crate) note: Option<String>,
}

/// The git feature's state in the core.
pub(crate) struct GitHub {
    /// `<changes dir>/git` (off without a changes folder).
    pub(crate) root: Option<PathBuf>,
    /// One commit, push or bring-in at a time.
    pub(crate) lock: tokio::sync::Mutex<()>,
    /// PR cards per session and selected repository.
    pub(crate) cards: Mutex<HashMap<String, CardCache>>,
    /// Nested roots discovered from this loaded session's file effects; every use revalidates them.
    pub(crate) repos: Mutex<HashMap<String, (PathBuf, BTreeSet<String>)>>,
    /// GitHub's public API: no request before this time (ms) after a low rate budget.
    pub(crate) api_resume_ms: Mutex<u64>,
}

impl GitHub {
    pub(crate) fn new(changes_dir: Option<&Path>) -> Self {
        Self {
            root: changes_dir.map(|d| d.join("git")),
            lock: tokio::sync::Mutex::new(()),
            cards: Mutex::new(HashMap::new()),
            repos: Mutex::new(HashMap::new()),
            api_resume_ms: Mutex::new(0),
        }
    }

    /// A private sub-folder of ours (`tmp` for message files, `gh` as gh's neutral folder), created on use.
    pub(crate) fn folder(&self, name: &str) -> Result<PathBuf, CoreError> {
        let root = self.root.as_ref().ok_or_else(|| {
            CoreError::new(
                ErrorCode::Internal,
                "git is off in this run (no data folder)",
            )
        })?;
        let dir = root.join(name);
        vw_platform::ensure_private_dir(root)
            .and_then(|()| vw_platform::ensure_private_dir(&dir))
            .map_err(|e| {
                CoreError::new(
                    ErrorCode::Internal,
                    format!("cannot prepare {}: {e}", dir.display()),
                )
            })?;
        Ok(dir)
    }

    pub(crate) fn card(&self, session: &str, root: &Path) -> CardCache {
        self.cards
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&format!("{session}:{}", root.display()))
            .cloned()
            .unwrap_or_default()
    }
}

#[allow(clippy::needless_pass_by_value)] // used as `.map_err(git_error)`
/// Maps a vw-git error to the command error (`blocked`/`rejected` as server codes, so the UI shows the text).
pub(crate) fn git_error(error: GitError) -> CoreError {
    let (code, server) = match &error {
        GitError::Blocked(_) => (ErrorCode::Denied, Some("blocked")),
        GitError::Stale(_) => (ErrorCode::Stale, None),
        GitError::Rejected(_) => (ErrorCode::ServerError, Some("rejected")),
        GitError::NotFound(_) => (ErrorCode::NotFound, None),
        _ => (ErrorCode::Internal, None),
    };
    let mut out = CoreError::new(code, error.to_string());
    out.server_code = server.map(str::to_owned);
    out
}

/// What the core knows about a session's git place.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    /// The working tree root.
    pub(crate) root: PathBuf,
    /// Its git dir.
    pub(crate) git_dir: PathBuf,
    /// The main repository's root (the copy's source; the root itself for a folder session).
    pub(crate) main_root: PathBuf,
    /// Copy or folder.
    pub(crate) location: GitLocation,
    /// A copy's base branch.
    pub(crate) base: Option<String>,
    /// A copy's own branch.
    pub(crate) own_branch: Option<String>,
    /// The session title.
    pub(crate) title: String,
    /// The agent's turn is running.
    pub(crate) running: bool,
}

impl Core {
    /// Executes one `git` request.
    ///
    /// # Errors
    /// `notFound` (unknown session, no repository), `denied` with server code `blocked` (the action is not
    /// possible here; the message says why), `stale`, `serverError`/`rejected` (git or the forge said no),
    /// `confirmationDeclined`, `internal`.
    pub async fn git(&self, origin: Origin, request: GitRequest) -> Result<GitResult, CoreError> {
        match request {
            GitRequest::Status { session_id, repo } => {
                self.git_status(&session_id, repo.as_deref()).await
            }
            GitRequest::PrepareCommit { session_id, repo } => {
                self.git_prepare(&session_id, repo.as_deref()).await
            }
            GitRequest::DraftMessage {
                session_id,
                repo,
                paths,
            } => self.git_draft(&session_id, repo.as_deref(), paths).await,
            GitRequest::Commit {
                session_id,
                repo,
                paths,
                message,
                branch,
                expected_head,
                run_hooks,
            } => {
                let expected = Expected {
                    head: expected_head,
                    branch,
                };
                self.git_commit(
                    &session_id,
                    repo.as_deref(),
                    paths,
                    message,
                    expected,
                    run_hooks,
                )
                .await
            }
            GitRequest::Settings { session_id, repo } => {
                let (_, place) = self.git_selected_place(&session_id, repo.as_deref())?;
                Ok(GitResult::Settings {
                    settings: self.git_settings(&place.main_root),
                })
            }
            GitRequest::SetSettings {
                session_id,
                repo,
                instruction,
                credit,
                run_hooks,
            } => {
                let (_, place) = self.git_selected_place(&session_id, repo.as_deref())?;
                let mut settings = self.git_settings(&place.main_root);
                if let Some(text) = instruction {
                    settings.instruction = text.chars().take(2000).collect();
                }
                if let Some(on) = credit {
                    settings.credit = on;
                }
                if let Some(on) = run_hooks {
                    settings.run_hooks = on;
                }
                crate::git_settings::save(&self.shared.git, &place.main_root, &settings)?;
                Ok(GitResult::Settings { settings })
            }
            other => self.git_publish(origin, other).await,
        }
    }

    /// The session's git place, or why there is none.
    pub(crate) fn git_place(&self, session_id: &str) -> Result<Place, CoreError> {
        let view = self
            .shared
            .store
            .view(session_id)
            .ok_or_else(|| CoreError::not_loaded(session_id))?;
        let session = facts(&view.state);
        let folder = session
            .cwd
            .clone()
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "the session has no folder yet"))?;
        let located = state::locate(&folder).map_err(git_error)?;
        let Some((root, git_dir)) = located else {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                "this session's folder is not in a git repository",
            ));
        };
        let record = self
            .shared
            .worktrees
            .records()
            .ok()
            .and_then(|r| r.get(session_id))
            .filter(|r| r.decision != Some(vw_worktrees::Decision::Removed));
        let (location, base, own_branch, main_root) = match record {
            Some(r) => (
                GitLocation::Copy,
                Some(r.base_name.clone()),
                Some(r.branch.clone()),
                r.repo_root.clone(),
            ),
            None => (GitLocation::Folder, None, None, root.clone()),
        };
        // A base given as a commit id (a detached start) is not a branch to compare with.
        let base = base.filter(|b| {
            vw_git::names::valid_ref_name(b)
                && !(b.len() >= 40 && b.chars().all(|c| c.is_ascii_hexdigit()))
        });
        Ok(Place {
            root,
            git_dir,
            main_root,
            location,
            base,
            own_branch,
            title: session.title.clone().unwrap_or_default(),
            running: session.running.is_some(),
        })
    }

    pub(crate) fn git_settings(&self, main_root: &Path) -> GitSettings {
        crate::git_settings::load(&self.shared.git, main_root)
    }

    async fn git_status(
        &self,
        session_id: &str,
        repo: Option<&str>,
    ) -> Result<GitResult, CoreError> {
        let repos = self.git_repos(session_id)?;
        let selected = repo.map(str::to_owned).or_else(|| {
            crate::git_discovery::default_root(repos.iter().map(|(r, _)| r.root.as_str()))
        });
        let place = match repos
            .iter()
            .find(|(r, _)| Some(&r.root) == selected.as_ref())
        {
            Some((_, place)) => place.clone(),
            None if repo.is_some() => {
                return Err(CoreError::new(
                    ErrorCode::NotFound,
                    "the selected repository was not found in this session's file changes",
                ));
            }
            None => {
                return Ok(GitResult::Status {
                    status: GitStatusView {
                        session_id: session_id.to_owned(),
                        repos: Vec::new(),
                        repo: None,
                        available: false,
                        reason: Some(self.git_place(session_id).err().map_or_else(
                            || {
                                "this session's recorded file changes have no git repository"
                                    .to_owned()
                            },
                            |error| error.message,
                        )),
                        location: None,
                        state: None,
                        uncommitted: None,
                        to_push: None,
                        blocker: None,
                        running: false,
                        pr: None,
                    },
                });
            }
        };
        let p = place.clone();
        let (state, listed, to_push) = blocking(move || {
            let state = state::read_state(&p.root, &p.git_dir, p.base.as_deref());
            let listed = commit::list(&p.root);
            let to_push = state.as_ref().ok().and_then(|state| {
                vw_git::push::plan(&p.root, state, None)
                    .ok()
                    .map(|plan| plan.commits)
            });
            (state, listed, to_push)
        })
        .await?;
        let state = state.map_err(git_error)?;
        let pr = self
            .shared
            .git
            .card(session_id, &place.root)
            .card
            .filter(|card| Some(card.head.as_str()) == state.branch());
        let (uncommitted, mut blocker) = match listed {
            Ok(files) => (Some(u32::try_from(files.len()).unwrap_or(u32::MAX)), None),
            Err(GitError::Blocked(why)) => (None, Some(why)),
            Err(other) => return Err(git_error(other)),
        };
        if blocker.is_none() {
            blocker = state_blocker(&state);
        }
        Ok(GitResult::Status {
            status: GitStatusView {
                session_id: session_id.to_owned(),
                repos: repos.into_iter().map(|(r, _)| r).collect(),
                repo: selected,
                available: true,
                reason: None,
                location: Some(place.location),
                state: Some(state),
                uncommitted,
                to_push,
                blocker,
                running: place.running,
                pr,
            },
        })
    }

    /// The session's folder as Vibe recorded it (`None` before it has one).
    fn session_folder(&self, session_id: &str) -> Option<std::path::PathBuf> {
        let view = self.shared.store.view(session_id)?;
        facts(&view.state).cwd.clone()
    }

    async fn git_prepare(
        &self,
        session_id: &str,
        repo: Option<&str>,
    ) -> Result<GitResult, CoreError> {
        let (_, place) = self.git_selected_place(session_id, repo)?;
        let settings = self.git_settings(&place.main_root);
        let p = place.clone();
        let (state, listed, author, hooks) = blocking(move || {
            (
                state::read_state(&p.root, &p.git_dir, None),
                commit::list(&p.root),
                commit::author(&p.root),
                vw_git::hooks::hooks(&p.root),
            )
        })
        .await?;
        let state = state.map_err(git_error)?;
        let mut blocker = state_blocker(&state);
        let files = match listed {
            Ok(files) => files,
            Err(GitError::Blocked(why)) => {
                blocker = Some(why);
                Vec::new()
            }
            Err(other) => return Err(git_error(other)),
        };
        if place.running && blocker.is_none() {
            blocker = Some(
                "Vibe is still working in this session. Commit when the turn has finished."
                    .to_owned(),
            );
        }
        let (author, author_problem) = match author {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e.to_string())),
        };
        // A session folder inside a larger repository commits in that whole repository; say so.
        let enclosing = self.session_folder(session_id).is_some_and(|folder| {
            let real = |p: &std::path::Path| {
                p.canonicalize()
                    .ok()
                    .map(|r| crate::changes_facts::path_key(&r))
            };
            real(&folder) != real(&place.root)
        });
        let note = (place.location == GitLocation::Folder).then(|| {
            if enclosing {
                format!(
                    "This commits on your branch in the repository {}, which contains this session's folder. The list \
                     is every change git sees in that whole repository, also outside this folder; untick what should \
                     not go in.",
                    crate::git_publish::folder_label(&place)
                )
            } else {
                "This commits on your branch in your own folder. The list is every change git sees there, also \
                 changes made outside this session; untick what should not go in."
                    .to_owned()
            }
        });
        Ok(GitResult::CommitPlan {
            plan: CommitPlanView {
                session_id: session_id.to_owned(),
                location: place.location,
                branch: state.branch().unwrap_or("").to_owned(),
                head: state.commit.clone(),
                files,
                author,
                author_problem,
                hooks: hooks.map_err(git_error)?,
                run_hooks: settings.run_hooks,
                credit: settings.credit,
                instruction: settings.instruction.clone(),
                note,
                blocker,
            },
        })
    }

    async fn git_commit(
        &self,
        session_id: &str,
        repo: Option<&str>,
        paths: Vec<String>,
        message: String,
        expected: Expected,
        run_hooks: bool,
    ) -> Result<GitResult, CoreError> {
        let (selected, place) = self.git_selected_place(session_id, repo)?;
        if place.running {
            return Err(git_error(GitError::Blocked(
                "Vibe is still working in this session. Commit when the turn has finished."
                    .to_owned(),
            )));
        }
        let _one = self.shared.git.lock.lock().await;
        self.git_validate_place(session_id, &place)?;
        // What the review shows now: committed files whose content is still what was committed leave it.
        let before = self.review_blobs(session_id).await;
        let scratch = self.shared.git.folder("tmp")?;
        let p = place.clone();
        let wanted = paths.clone();
        let done = blocking(move || {
            commit::commit(
                &p.root, &p.git_dir, &wanted, &message, &expected, &scratch, run_hooks,
            )
        })
        .await?
        .map_err(git_error)?;
        let review_paths = if selected == "." {
            paths.clone()
        } else {
            paths
                .iter()
                .map(|path| format!("{selected}/{path}"))
                .collect()
        };
        let kept = self
            .keep_committed(session_id, &review_paths, &before)
            .await;
        tracing::info!(
            session = session_id,
            files = paths.len(),
            kept,
            hooks = run_hooks,
            "git commit"
        );
        crate::changes::refresh_and_emit(&self.shared, session_id).await;
        Ok(GitResult::Committed {
            commit: done.id,
            output: done.output,
            kept,
        })
    }

    /// The review's files as `path -> blob` (empty when the review is not a git review).
    async fn review_blobs(&self, session_id: &str) -> HashMap<String, Option<String>> {
        let read = crate::changes_view::ChangesRequest::Read {
            session_id: session_id.to_owned(),
        };
        match self.changes(Origin::Ui, read).await {
            Ok(crate::changes_view::ChangesResult::View { view }) => {
                view.files.into_iter().map(|f| (f.path, f.blob)).collect()
            }
            _ => HashMap::new(),
        }
    }

    /// Keeps, in the review, each committed file whose blob is unchanged since before the commit.
    async fn keep_committed(
        &self,
        session_id: &str,
        paths: &[String],
        before: &HashMap<String, Option<String>>,
    ) -> u32 {
        let now = self.review_blobs(session_id).await;
        let mut kept = 0;
        for path in paths {
            let (Some(old), Some(new)) = (before.get(path), now.get(path)) else {
                continue;
            };
            if old != new {
                continue;
            }
            let keep = crate::changes_view::ChangesRequest::Keep {
                session_id: session_id.to_owned(),
                target: vw_changes::Target::File {
                    path: path.clone(),
                    blob: new.clone(),
                },
            };
            if self.changes(Origin::Ui, keep).await.is_ok() {
                kept += 1;
            }
        }
        kept
    }
}

/// Why commits are not possible from the branch state (detached, an operation in progress).
pub(crate) fn state_blocker(state: &BranchState) -> Option<String> {
    if state.branch().is_none() {
        return Some(
            "This folder is not on a branch (detached HEAD). Switch to a branch first.".to_owned(),
        );
    }
    state.in_progress.map(|op| {
        format!(
            "git has {} in progress in this folder. Finish or abort it in your git tool first.",
            op.words()
        )
    })
}
