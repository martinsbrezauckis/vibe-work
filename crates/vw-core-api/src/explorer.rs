//! Files operations for exact picked, project and recorded session folders. Depends on bounded readers and the git lock.
//! Called by `GitBrowser`; no operation creates a session or sends anything to Vibe.
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::explorer_view::{ExplorerRequest, ExplorerResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};

/// Prefix for file and Git actions before a session exists.
pub(crate) const WORKSPACE: &str = "files-workspace:";

/// Cancellation belongs to the folder; different windows never cancel each other's repositories.
#[derive(Default)]
pub(crate) struct Searches(Mutex<HashMap<PathBuf, Arc<AtomicBool>>>);
impl Searches {
    pub(crate) fn replace(&self, root: &Path) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        let mut searches = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        searches.retain(|_, token| Arc::strong_count(token) > 1);
        if let Some(old) = searches.insert(root.to_owned(), token.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        token
    }
}

impl Core {
    /// Revalidates an exact known root before any filesystem access.
    pub(crate) fn explorer_folder(&self, folder: &str) -> Result<PathBuf, CoreError> {
        // `Path::starts_with` compares components without resolving `..`, so `<root>/..` would count as
        // inside `<root>`. Clean the hops lexically first (still no filesystem access before the
        // membership check), then compare; `safe_path` below rejects symlinked components.
        //
        // Both sides are compared in their plain spelling (`within`): on Windows a picked folder is kept in the
        // `\\?\C:\...` form `canonicalize` gives, while the window asks with the folder dialog's `C:\...`, so
        // without it a picked folder with no session yet (a new chat's folder) was refused. Names still compare
        // exactly (case included), so a spelling of another folder never matches.
        let path = lexical_clean(Path::new(folder))
            .map(|path| vw_platform::plain_path(&path))
            .ok_or_else(denied)?;
        if !path.is_absolute() {
            return Err(denied());
        }
        let path = self.known_folder(path).ok_or_else(denied)?;
        vw_git::browser::safe_path(Path::new(""), &path)
            .filter(|p| p.is_dir())
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::NotFound,
                    "This folder could not be read. Choose it again.",
                )
            })
    }
    /// The folder to read for `path` (lexically clean, plain, absolute) when it is a known root or lies inside one:
    /// a folder picked in this run, the New chat folder restored after a restart, a session's folder or a project's
    /// main folder. Compared as written, so a path outside every known folder is refused without any disk access.
    ///
    /// macOS and Linux: a root may be spelled through a symbolic link the user chose (macOS `/tmp/x` is
    /// `/private/tmp/x`; a linked project folder), and the link check that follows refuses every link on the way,
    /// so such a folder was refused although it was chosen. The root's own spelling is therefore read at its real
    /// location (a pick: the folder resolved at the pick; a session or project folder: resolved now, after it was
    /// recognised), and the part below the root is kept as written, so a link inside the folder is still refused.
    /// Windows keeps the plain-spelling comparison (`within`); a root there is used as written.
    fn known_folder(&self, path: PathBuf) -> Option<PathBuf> {
        let shared = &self.shared;
        if shared
            .gate
            .picked_roots()
            .iter()
            .any(|root| within(&path, root))
            || shared
                .new_chat
                .restored_folder()
                .is_some_and(|root| within(&path, &root))
        {
            return Some(path);
        }
        #[cfg(not(windows))]
        for (spelling, canonical) in shared.gate.picked_spellings() {
            if let Ok(rest) = path.strip_prefix(&spelling) {
                return Some(canonical.join(rest));
            }
        }
        let mut roots: Vec<PathBuf> = shared
            .names
            .session_folders(&shared.store)
            .into_iter()
            .map(|(_, cwd)| PathBuf::from(cwd))
            .collect();
        shared.projects.store.read(|store| {
            roots.extend(
                store
                    .projects()
                    .iter()
                    .filter_map(|p| p.main_folder.as_ref().map(PathBuf::from)),
            );
        });
        // The longest (most specific) known root that holds the path.
        let root = roots
            .into_iter()
            .map(|root| vw_platform::plain_path(&root))
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())?;
        Some(at_real_location(&root, path))
    }

    /// Returns a revalidated workspace root, or None for a real session key.
    pub(crate) fn workspace_folder(&self, key: &str) -> Result<Option<PathBuf>, CoreError> {
        key.strip_prefix(WORKSPACE)
            .map(|folder| self.explorer_folder(folder))
            .transpose()
    }
    pub(crate) async fn explore(
        &self,
        folder: String,
        request: ExplorerRequest,
    ) -> Result<ExplorerResult, CoreError> {
        let root = self.explorer_folder(&folder)?;
        let cancel = if matches!(
            request,
            ExplorerRequest::Search { .. } | ExplorerRequest::Cancel {}
        ) {
            Some(self.shared.files.searches.replace(&root))
        } else {
            None
        };
        let _write = if matches!(
            request,
            ExplorerRequest::Revert { .. }
                | ExplorerRequest::Review {}
                | ExplorerRequest::EnableHistory { .. }
        ) {
            Some(self.shared.git.lock.lock().await)
        } else {
            None
        };
        let scratch = self.shared.changes.dir.clone();
        crate::worktrees::blocking(move || match request {
            ExplorerRequest::Folder {
                path,
                show_ignored,
                offset,
            } => crate::explorer_read::folder(&root, &path, show_ignored, offset),
            ExplorerRequest::Status {} => {
                let status = vw_git::browser_status::read(&root).ok();
                let note = (status.is_none() && root.join(".git").exists()).then(|| "Files still work; history needs Git. Check that Git can read this folder, then Refresh.".to_owned());
                let commits = crate::explorer_read::git(&root, &["rev-list", "--count", "HEAD"], 64).ok().and_then(|bytes| String::from_utf8_lossy(&bytes).trim().parse().ok());
                let latest = match crate::explorer_history::history(&root, None, "", None) { Ok(ExplorerResult::History { versions, .. }) => versions.into_iter().next(), _ => None };
                Ok(ExplorerResult::Status { status, note, commits, latest })
            },
            ExplorerRequest::Index {} => Ok(crate::explorer_read::index(&root)),
            ExplorerRequest::Search { query, path } => {
                crate::explorer_read::search(&root, &query, path.as_deref(), cancel.as_deref())
            }
            ExplorerRequest::Cancel {} => Ok(ExplorerResult::Done {}),
            ExplorerRequest::History {
                cursor,
                query,
                path,
            } => crate::explorer_history::history(&root, cursor.as_deref(), &query, path.as_deref()),
            ExplorerRequest::ReadAt { id, path } => crate::explorer_history::read_at(&root, &id, &path),
            ExplorerRequest::Version { id } => crate::explorer_history::version(&root, &id),
            ExplorerRequest::Revert {
                head,
                fingerprint,
                delete_paths,
            } => crate::explorer_history::revert(
                &root,
                scratch
                    .as_deref()
                    .ok_or_else(crate::explorer_read::failed)?,
                &head,
                &fingerprint,
                &delete_paths,
            ),
            ExplorerRequest::Review {} => workspace_review(&root, scratch.as_deref().ok_or_else(crate::explorer_read::failed)?),
            ExplorerRequest::PlanHistory {} => vw_changes::plan_init(&root)
                .map(|plan| ExplorerResult::InitPlan { plan })
                .map_err(crate::changes_view::core_error),
            ExplorerRequest::EnableHistory {
                confirmed,
                identity,
            } => {
                if !confirmed {
                    return Err(CoreError::new(
                        ErrorCode::InvalidParams,
                        "Confirm turning on version history first.",
                    ));
                }
                vw_git::identity::validate(&identity).map_err(crate::git_cmd::git_error)?;
                vw_changes::init_repository_with_identity(&root, &identity.name, &identity.email)
                    .map_err(crate::changes_view::core_error)?;
                Ok(ExplorerResult::Done {})
            }
        })
        .await?
    }
}

/// `path` (inside the known `root`) with the root replaced by its real location (symbolic links resolved), the part
/// below the root kept as written; `path` unchanged when the root has no link or cannot be resolved (the link check
/// then answers).
#[cfg(not(windows))]
fn at_real_location(root: &Path, path: PathBuf) -> PathBuf {
    match (root.canonicalize(), path.strip_prefix(root)) {
        (Ok(real), Ok(rest)) if real != root => real.join(rest),
        _ => path,
    }
}

/// Windows: a known root is used as written (see `Core::known_folder`).
#[cfg(windows)]
fn at_real_location(_root: &Path, path: PathBuf) -> PathBuf {
    path
}

fn denied() -> CoreError {
    CoreError::new(
        ErrorCode::Denied,
        "Choose a folder first to browse its files.",
    )
}

/// True when `path` (already lexically clean and plain) is `root` or lies inside it, comparing whole names in the
/// plain spelling of `root` (Windows: without the `\\?\` prefix when that names the same folder). No disk access.
pub(crate) fn within(path: &Path, root: &Path) -> bool {
    path.starts_with(vw_platform::plain_path(root))
}

/// Removes `.` and resolves `..` against the preceding component without touching the disk; None when a
/// `..` would climb above the path's root.
fn lexical_clean(path: &Path) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                if !clean.pop() {
                    return None;
                }
            }
            std::path::Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    Some(clean)
}

fn workspace_review(root: &Path, scratch: &Path) -> Result<ExplorerResult, CoreError> {
    let (_, diff) = crate::explorer_history::review(root, scratch)?;
    let head = crate::explorer_read::git(root, &["rev-parse", "--verify", "HEAD"], 256)
        .unwrap_or_default();
    Ok(ExplorerResult::Review {
        head: String::from_utf8_lossy(&head).trim().to_owned(),
        view: crate::changes_view::ChangesView {
            session_id: format!("{WORKSPACE}{}", root.display()),
            mode: crate::changes_view::ChangesMode::Uncommitted,
            folder: Some(root.to_string_lossy().into_owned()),
            repo_root: Some(root.to_string_lossy().into_owned()),
            scope: Some(String::new()),
            reason: None,
            baseline_lost: false,
            baseline_at_ms: None,
            files: diff.files,
            fingerprint: diff.fingerprint,
            notes: diff.notes,
            turns: Vec::new(),
            offer_history: false,
            watching: false,
            took_ms: 0,
        },
    })
}
