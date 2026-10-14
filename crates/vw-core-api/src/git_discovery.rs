//! Bounded Git repository discovery from session file effects, the session folder's direct subfolders and previously
//! validated roots.
//! Depends on: Changes facts, vw-git, the Git core state. Called by every Git request and history initialization.
//! Invariants: roots are revalidated in the session folder; the tree is never walked (only the folder's own entries
//! are listed, bounded); client strings only select a discovered root.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use std::sync::PoisonError;

use vw_git::state;

use crate::changes_facts::{FileTouch, facts};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::git_cmd::Place;
use crate::git_view::{GitLocation, GitRepoView};

impl Core {
    /// Rebuild the selectable repositories from the session's recorded file effects on every request.
    /// A client-supplied root is matched against this list, never joined to a filesystem path.
    pub(crate) fn git_repos(
        &self,
        session_id: &str,
    ) -> Result<Vec<(GitRepoView, Place)>, CoreError> {
        let view = self
            .shared
            .store
            .view(session_id)
            .ok_or_else(|| CoreError::not_loaded(session_id))?;
        let session = facts(&view.state);
        let cwd = session
            .cwd
            .as_ref()
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "the session has no folder yet"))?;
        if cwd
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
            || cwd.canonicalize().is_ok_and(|real| {
                crate::changes_facts::path_key(&real) != crate::changes_facts::path_key(cwd)
            })
        {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "the session folder is a symbolic link",
            ));
        }
        let mut found = BTreeMap::new();
        if let Ok(place) = self.git_place(session_id) {
            found.insert(".".to_owned(), place);
        }
        let prior = self
            .shared
            .git
            .repos
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .filter(|(folder, _)| {
                crate::changes_facts::path_key(folder) == crate::changes_facts::path_key(cwd)
            })
            .map(|(_, roots)| roots.clone())
            .unwrap_or_default();
        for label in prior {
            if let Some(place) = nested_place(
                cwd,
                &label,
                session.title.as_ref(),
                session.running.is_some(),
            ) {
                found.insert(label, place);
            }
        }
        add_touched_repos(
            cwd,
            &session.touches,
            session.title.as_ref(),
            session.running.is_some(),
            &mut found,
        );
        add_child_repos(
            cwd,
            session.title.as_ref(),
            session.running.is_some(),
            &mut found,
        );
        self.shared
            .git
            .repos
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                session_id.to_owned(),
                (
                    cwd.clone(),
                    found
                        .keys()
                        .filter(|root| root.as_str() != ".")
                        .cloned()
                        .collect(),
                ),
            );
        Ok(found
            .into_iter()
            .map(|(root, place)| {
                let branch = state::read_state(&place.root, &place.git_dir, place.base.as_deref())
                    .ok()
                    .and_then(|state| state.branch().map(str::to_owned));
                // Always the repository's own folder name, also for "." (a session in a subfolder
                // of a larger repository works in that repository, not in a repository named after the subfolder).
                let name = place
                    .root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                (GitRepoView { name, root, branch }, place)
            })
            .collect())
    }

    pub(crate) fn git_selected_place(
        &self,
        session_id: &str,
        repo: Option<&str>,
    ) -> Result<(String, Place), CoreError> {
        let places = self.git_repos(session_id)?;
        if repo.is_none() && places.is_empty() {
            // No repository at all: keep the session folder's own reason (not a repository, ignored by
            // its parent repository, ...) and its code instead of a generic "not found".
            self.git_place(session_id)?;
        }
        let selected = repo
            .map(str::to_owned)
            .or_else(|| default_root(places.iter().map(|(r, _)| r.root.as_str())));
        places
            .into_iter()
            .find(|(r, _)| Some(&r.root) == selected.as_ref())
            .map(|(r, p)| (r.root, p))
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::NotFound,
                    "the selected repository was not found in this session's file changes",
                )
            })
    }

    pub(crate) fn git_bring_repo(
        &self,
        session_id: &str,
        repo: Option<&str>,
    ) -> Result<(), CoreError> {
        let (selected, _) = self.git_selected_place(session_id, repo)?;
        if selected != "." {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "Bring into the base is only available for the session repository",
            ));
        }
        Ok(())
    }

    pub(crate) fn git_validate_place(
        &self,
        session_id: &str,
        place: &Place,
    ) -> Result<(), CoreError> {
        if self
            .git_repos(session_id)?
            .iter()
            .any(|(_, current)| current.root == place.root && current.git_dir == place.git_dir)
        {
            Ok(())
        } else {
            Err(CoreError::new(
                ErrorCode::Stale,
                "the selected repository changed since the plan was shown",
            ))
        }
    }
}

/// Inspect only ancestors of recorded file effects; never list directory contents.
fn add_touched_repos(
    cwd: &Path,
    touches: &[FileTouch],
    title: Option<&String>,
    running: bool,
    found: &mut BTreeMap<String, Place>,
) {
    let mut inspected = BTreeSet::new();
    for touch in touches.iter().rev() {
        let relative = touch
            .path
            .strip_prefix(cwd)
            .map(Path::to_path_buf)
            .or_else(|_| {
                crate::changes_facts::path_key(&touch.path)
                    .strip_prefix(crate::changes_facts::path_key(cwd))
                    .map(Path::to_path_buf)
            });
        let Ok(relative) = relative else {
            continue;
        };
        if !safe_relative(cwd, &relative, 9) {
            continue;
        }
        let effect_path = cwd.join(&relative);
        let mut candidate = effect_path.parent();
        for _ in 0..8 {
            let Some(dir) = candidate else { break };
            if dir == cwd {
                break;
            }
            let Ok(rel) = dir.strip_prefix(cwd) else {
                break;
            };
            let label = rel.to_string_lossy().replace('\\', "/");
            let label = if cfg!(windows) {
                label.to_lowercase()
            } else {
                label
            };
            if found.contains_key(&label) {
                candidate = dir.parent();
                continue;
            }
            if !inspected.insert(dir.to_path_buf()) {
                candidate = dir.parent();
                continue;
            }
            if let Some(place) = nested_place(cwd, &label, title, running) {
                found.insert(label, place);
            }
            candidate = dir.parent();
        }
    }
}

/// The session folder's direct subfolders that hold a `.git` (a repository the agent made with Bash leaves no
/// file effect, so its git bar was missing and the handover named buttons that were not there). Lists only the
/// folder's own entries, at most `MAX_CHILDREN`, and every candidate passes `nested_place`'s checks.
fn add_child_repos(
    cwd: &Path,
    title: Option<&String>,
    running: bool,
    found: &mut BTreeMap<String, Place>,
) {
    const MAX_CHILDREN: usize = 512;
    let Ok(entries) = std::fs::read_dir(cwd) else {
        return;
    };
    for entry in entries.flatten().take(MAX_CHILDREN) {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let label = if cfg!(windows) {
            name.to_lowercase()
        } else {
            name
        };
        if found.contains_key(&label) || entry.path().join(".git").symlink_metadata().is_err() {
            continue;
        }
        if let Some(place) = nested_place(cwd, &label, title, running) {
            found.insert(label, place);
        }
    }
}

fn safe_relative(cwd: &Path, relative: &Path, max_depth: usize) -> bool {
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty()
        || parts.len() > max_depth
        || parts.iter().any(|part| {
            !matches!(part, Component::Normal(_))
                || matches!(part, Component::Normal(name) if
            [".git", "node_modules", "target", "dist", "build", "out"]
                .contains(&name.to_string_lossy().to_ascii_lowercase().as_str()))
        })
    {
        return false;
    }
    let mut path = cwd.to_path_buf();
    for part in parts {
        path.push(part.as_os_str());
        if path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return false;
        }
    }
    true
}

fn nested_place(cwd: &Path, label: &str, title: Option<&String>, running: bool) -> Option<Place> {
    let relative = Path::new(label);
    if !safe_relative(cwd, relative, 8) {
        return None;
    }
    let dir = cwd.join(relative);
    let marker = dir.join(".git");
    if marker.symlink_metadata().ok()?.file_type().is_symlink() {
        return None;
    }
    let (root, git_dir) = state::locate(&dir).ok()??;
    // The repository's data must live in the session folder too: a `.git` file (gitdir:) or a `commondir` entry can
    // point git at another repository on disk, whose branch a Commit would write and whose remote a Push would use
    // with the user's sign-in (a confused deputy for an agent that cannot push itself). Submodules and worktrees
    // whose data is inside the folder stay discoverable.
    let real_cwd = cwd.canonicalize().ok()?;
    let inside = |path: &Path| {
        path.canonicalize().is_ok_and(|real| {
            crate::changes_facts::path_key(&real)
                .starts_with(crate::changes_facts::path_key(&real_cwd))
        })
    };
    if !inside(&git_dir) {
        return None;
    }
    // The common dir as git itself resolves it (no parser of ours that could differ from git's).
    let common = vw_changes::git::Call::new(&dir, vw_changes::git::Index::User)
        .args(["rev-parse", "--git-common-dir"])
        .timeout(std::time::Duration::from_secs(10))
        .run()
        .ok()?;
    let common = String::from_utf8_lossy(&common)
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    let common = dir.join(common);
    if !inside(&common) {
        return None;
    }
    let data_dirs = [git_dir.clone(), common];
    // Symlinks inside the git data (refs, objects, HEAD, config, ...) could still lead git to another repository.
    if data_dirs.iter().any(|dir| links_out(dir)) {
        return None;
    }
    let same_root = root
        .canonicalize()
        .ok()
        .zip(dir.canonicalize().ok())
        .is_some_and(|(a, b)| {
            crate::changes_facts::path_key(&a) == crate::changes_facts::path_key(&b)
        });
    if !same_root {
        return None;
    }
    Some(Place {
        root,
        git_dir,
        main_root: dir,
        location: GitLocation::Folder,
        base: None,
        own_branch: None,
        title: title.cloned().unwrap_or_default(),
        running,
    })
}

/// The repository a request without `repo` means: the session folder's own (".") whenever it is one. Labels are
/// sorted, and a nested folder named e.g. "-x" or " x" sorts before "."; it must never become the default target.
pub(crate) fn default_root<'a>(roots: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut first = None;
    for root in roots {
        if root == "." {
            return Some(".".to_owned());
        }
        if first.is_none() {
            first = Some(root.to_owned());
        }
    }
    first
}

/// True when a git data folder holds a symbolic link where git keeps a repository's state: the top-level
/// entries, or anything under `refs` (bounded walk). A real repository made by `git init`/`clone` has none.
fn links_out(git_dir: &Path) -> bool {
    const ENTRIES: [&str; 13] = [
        "HEAD",
        "config",
        "refs",
        "objects",
        "packed-refs",
        "index",
        "logs",
        "info",
        "hooks",
        "worktrees",
        "shallow",
        "reftable",
        "commondir",
    ];
    let link = |path: &Path| {
        path.symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
    };
    if ENTRIES.iter().any(|entry| link(&git_dir.join(entry))) {
        return true;
    }
    // One level of `objects` (the fan-out folders, pack, info): bounded, so a large repository is still offered.
    if let Ok(entries) = std::fs::read_dir(git_dir.join("objects")) {
        for entry in entries.flatten().take(1024) {
            if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
                return true;
            }
        }
    }
    let mut stack = vec![git_dir.join("refs"), git_dir.join("logs")];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 20_000 {
                // Too many refs to check cheaply: not offered rather than unchecked.
                return true;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_symlink() => return true,
                Ok(kind) if kind.is_dir() => stack.push(path),
                _ => {}
            }
        }
    }
    false
}
