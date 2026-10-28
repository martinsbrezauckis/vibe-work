//! Folders the user picked in a native dialog, the only folders that may be trusted.
//!
//! Depends on: std only (filesystem canonicalisation). Called by: gate (`register_picked_folder`),
//! privileged (`workspace/trust/decision`, including the repository root of `trust_repo`),
//! `agent_config` (`trustWorkspace`, `workspaceRoots`, an existing `worktree`).
//! Invariants:
//! - Folders are stored canonicalised (symlinks resolved), and a candidate path is canonicalised
//!   the same way before comparison, so `..` hops and symlinks out of a picked folder do not match.
//! - A candidate that does not exist, is relative, or cannot be resolved never matches.
//! - The spelling a folder was picked in (the dialog's, the Debug API's) is kept beside its canonical path when the
//!   two differ (macOS `/tmp/x` is `/private/tmp/x`; a symlinked project folder), so the Files tab can recognise the
//!   window's spelling without touching the disk ([`PickedFolders::spellings`]). It is never used for trust.
//!
//! Limitation (documented, not solved here): the path is checked and later used by Vibe as the
//! string the caller sent; a local process that can rewrite the picked folder's contents between
//! the check and Vibe's use could swap a directory for a symlink. Closing it needs Vibe to accept
//! a directory handle, which the protocol does not offer.

use std::path::{Path, PathBuf};

use crate::error::GateError;

/// The set of natively picked folders.
#[derive(Default)]
pub(crate) struct PickedFolders {
    /// Canonical paths.
    roots: Vec<PathBuf>,
    /// `(spelling as picked, canonical path)` for picks whose spelling differs from the canonical path.
    spellings: Vec<(PathBuf, PathBuf)>,
}

impl PickedFolders {
    /// Records a folder the user picked. It must exist and be a directory.
    pub(crate) fn register(&mut self, path: &Path) -> Result<(), GateError> {
        let shown = path.display().to_string();
        let canonical = path.canonicalize().map_err(|e| GateError::InvalidFolder {
            path: shown.clone(),
            reason: e.to_string(),
        })?;
        if !canonical.is_dir() {
            return Err(GateError::InvalidFolder {
                path: shown,
                reason: "not a directory".to_owned(),
            });
        }
        if path.is_absolute() && path != canonical {
            let spelling = (path.to_path_buf(), canonical.clone());
            if !self.spellings.contains(&spelling) {
                self.spellings.push(spelling);
            }
        }
        if !self.roots.contains(&canonical) {
            self.roots.push(canonical);
        }
        Ok(())
    }

    /// True when `candidate` is a picked folder or lies inside one.
    pub(crate) fn contains(&self, candidate: &str) -> bool {
        self.contains_path(Path::new(candidate))
    }

    /// The picked folders (canonical), for callers that must compare a path lexically before touching the disk
    /// (a path outside every allowed folder is refused without any filesystem access).
    pub(crate) fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    /// The picks whose spelling differs from their canonical path, as `(spelling, canonical)`, for the same lexical
    /// comparison as [`Self::roots`] in the spelling the window keeps.
    pub(crate) fn spellings(&self) -> Vec<(PathBuf, PathBuf)> {
        self.spellings.clone()
    }

    /// [`Self::contains`] for a path value.
    pub(crate) fn contains_path(&self, path: &Path) -> bool {
        if !path.is_absolute() {
            return false;
        }
        let Ok(real) = path.canonicalize() else {
            return false;
        };
        self.roots.iter().any(|picked| real.starts_with(picked))
    }
}

/// The git repository root Vibe would trust for `trust_repo` on `cwd`: the closest of `cwd` and
/// its ancestors that holds a `.git` directory with a `HEAD` file, after resolving symlinks.
///
/// Mirrors `find_git_repo_ancestor` in Vibe 2.26.0 (`vibe/core/trusted_folders.py`), except that
/// Vibe stops below the home directory and the filesystem root while this walks all the way up.
/// The walk is closest-first in both, so whenever Vibe finds a root this finds the same one; when
/// Vibe finds none it refuses `trust_repo` itself. `None` when `cwd` is relative or unresolvable.
pub(crate) fn repo_root(cwd: &str) -> Option<PathBuf> {
    let path = Path::new(cwd);
    if !path.is_absolute() {
        return None;
    }
    let real = path.canonicalize().ok()?;
    real.ancestors()
        .find(|dir| {
            let git = dir.join(".git");
            git.is_dir() && git.join("HEAD").is_file()
        })
        .map(Path::to_path_buf)
}
