//! The repository's commit hooks: the user's own Commit runs them as `git commit`
//! in a terminal does (per-project switch "Run this repository's commit hooks", on by default), and the dialog
//! names which ones will run. Every implicit git call keeps the private empty hooks folder; Push never runs
//! `pre-push`. A hook the agent edited is not checked separately (with full access the agent can run
//! any command anyway; in ask mode the edit comes through an approval and shows in Changes).
//!
//! Depends on: vw-changes (runner), vw-platform (`plain_path`). Called by: vw-core-api.
//! Invariant: read only; no hook runs here (`rev-parse` only).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vw_changes::git::{Call, Index};

use crate::error::GitError;

/// The hooks a commit with the switch on would run: the hooks folder (`core.hooksPath` or `.git/hooks`,
/// shown relative to the repository when inside it) and the commit hooks present in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HookInfo {
    /// Where they are.
    pub folder: String,
    /// Which commit hooks exist there (`pre-commit`, `prepare-commit-msg`, `commit-msg`, `post-commit`).
    pub hooks: Vec<String>,
}

/// The hooks git runs on a commit.
const COMMIT_HOOKS: &[&str] = &[
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
];

/// Reads which commit hooks the repository would run (no hook runs: `rev-parse` only).
///
/// # Errors
/// git failures.
pub fn hooks(root: &Path) -> Result<HookInfo, GitError> {
    let folder = hooks_folder(root)?;
    let hooks = COMMIT_HOOKS
        .iter()
        .filter(|name| runnable(&folder.join(name)))
        .map(|name| (*name).to_owned())
        .collect();
    // Compared in their plain forms: a root given as `canonicalize` answers it on Windows
    // (`\\?\C:\...`) and git's `C:\...` answer for a configured hooks folder name the same folder.
    let (plain_folder, plain_root) = (
        vw_platform::plain_path(&folder),
        vw_platform::plain_path(root),
    );
    let shown = plain_folder.strip_prefix(&plain_root).map_or_else(
        |_| folder.display().to_string(),
        |p| p.display().to_string(),
    );
    Ok(HookInfo {
        folder: shown,
        hooks,
    })
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    // Git for Windows runs a hook file that exists (no executable bit).
    path.is_file()
}

fn hooks_folder(root: &Path) -> Result<PathBuf, GitError> {
    let out = Call::new(root, Index::User)
        .with_repo_hooks()
        .args(["rev-parse", "--git-path", "hooks"])
        .timeout(Duration::from_secs(10))
        .run()?;
    let text = String::from_utf8_lossy(&out).trim().to_owned();
    Ok(if Path::new(&text).is_absolute() {
        PathBuf::from(text)
    } else {
        root.join(text)
    })
}
