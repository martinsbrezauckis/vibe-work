//! What the new-session panel needs to know about a folder's repository: where it is, which branch is
//! checked out, the local branches (the base choices), the worktrees it has, and whether a copy may be made
//! at all (the preflight).
//!
//! **The preflight**: making and checking a copy runs git commands in it (a checkout, a status),
//! and those can start programs that a repository's own config names: filter drivers (clean, smudge,
//! process) and an fsmonitor. Vibe Work makes copies only of repositories that name no such program in their
//! own config (`local`/`worktree` scope; included files count as `local`), so a copy never starts a program
//! the user did not install. The three canonical git-lfs commands are allowed: they run the user's own
//! installed `git-lfs`, exactly as the user's own `git` would. Hooks are not a reason: the copy is made with
//! hooks turned off, and `git worktree remove` runs none.
//!
//! Depends on: vw-changes (the hardened runner `git::Call`), `error`. Called by: vw-core-api (`options`,
//! `start`, before every Vibe inspect/removal), `inspect`.
//! Invariant: read only; every call reads config, refs or the worktree list, never the working tree.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vw_changes::git::{Call, Index};

use crate::error::{WorktreeError, clip};

const READ: Duration = Duration::from_secs(10);

/// The canonical git-lfs filter commands (`git lfs install` writes exactly these).
const LFS: &[(&str, &str)] = &[
    ("clean", "git-lfs clean -- %f"),
    ("smudge", "git-lfs smudge -- %f"),
    ("process", "git-lfs filter-process"),
];

/// What `HEAD` of the folder's checkout is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Head {
    /// On a branch.
    Branch {
        /// Short name (`main`).
        name: String,
    },
    /// Detached at a commit.
    Detached {
        /// The commit id.
        commit: String,
    },
    /// No commit yet (a copy needs a commit to start from).
    Unborn,
}

/// One local branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    /// Short name.
    pub name: String,
    /// The commit it points at.
    pub commit: String,
    /// Checked out in the folder's checkout.
    pub current: bool,
}

/// One working tree git lists for the repository (the main one first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// Its root folder.
    pub path: PathBuf,
    /// Its branch (short name), `None` when detached.
    pub branch: Option<String>,
}

/// A folder's repository as the worktree feature sees it.
#[derive(Debug, Clone)]
pub struct RepoInfo {
    /// Root of the folder's own working tree.
    pub root: PathBuf,
    /// The repository's common git directory (shared by all its worktrees).
    pub common_dir: PathBuf,
    /// The folder's `HEAD`.
    pub head: Head,
    /// Local branches, the current one first, then by name.
    pub branches: Vec<BranchInfo>,
    /// Every working tree of the repository.
    pub worktrees: Vec<WorktreeEntry>,
    /// Why a copy is not offered (`None` = offered).
    pub not_offered: Option<String>,
}

impl RepoInfo {
    /// Root of the main working tree (the first entry git lists), else the folder's own root.
    #[must_use]
    pub fn main_root(&self) -> &Path {
        self.worktrees.first().map_or(&self.root, |w| &w.path)
    }
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).trim().to_owned()
}

/// Reads the repository of `folder`. `Ok(None)` when it is not inside a git working tree (or is a bare
/// repository, or `.git` itself).
///
/// # Errors
/// `NotGit` when git refuses the folder (another user's repository), `Git` when git cannot run.
pub fn read(folder: &Path) -> Result<Option<RepoInfo>, WorktreeError> {
    if !folder.is_dir() {
        return Ok(None);
    }
    let out = Call::new(folder, Index::User)
        .args([
            "rev-parse",
            "--is-inside-work-tree",
            "--show-toplevel",
            "--git-common-dir",
        ])
        .timeout(READ)
        .output()?;
    if !out.ok() {
        if out
            .stderr
            .to_ascii_lowercase()
            .contains("dubious ownership")
        {
            return Err(WorktreeError::NotGit(
                "git refuses this folder: it belongs to another user (git's safe.directory check)"
                    .into(),
            ));
        }
        return Ok(None);
    }
    let printed = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = printed.lines();
    if lines.next() != Some("true") {
        return Ok(None);
    }
    let (Some(root), Some(common)) = (lines.next(), lines.next()) else {
        return Ok(None);
    };
    let root = PathBuf::from(root);
    // `--git-common-dir` may be relative to the folder git ran in.
    let common_dir = {
        let c = PathBuf::from(common);
        if c.is_absolute() { c } else { folder.join(c) }
    };
    let head = head(&root)?;
    let branches = branches(&root, &head)?;
    let worktrees = worktrees(&root)?;
    let not_offered = match &head {
        Head::Unborn => Some(
            "This repository has no commit yet, so there is nothing to copy. Make a first commit, or work in the folder itself."
                .to_owned(),
        ),
        _ => preflight(&root)?,
    };
    Ok(Some(RepoInfo {
        root,
        common_dir,
        head,
        branches,
        worktrees,
        not_offered,
    }))
}

fn head(root: &Path) -> Result<Head, WorktreeError> {
    let commit = Call::new(root, Index::User)
        .args(["rev-parse", "--verify", "-q", "HEAD^{commit}"])
        .timeout(READ)
        .output()?;
    if !commit.ok() {
        // A branch without a commit yet (or no HEAD at all).
        return Ok(Head::Unborn);
    }
    let sym = Call::new(root, Index::User)
        .args(["symbolic-ref", "-q", "HEAD"])
        .timeout(READ)
        .output()?;
    let full = text(&sym.stdout);
    Ok(
        match full.strip_prefix("refs/heads/").filter(|_| sym.ok()) {
            Some(name) => Head::Branch {
                name: name.to_owned(),
            },
            // Detached (or a symbolic ref outside `refs/heads`): treated as its commit.
            None => Head::Detached {
                commit: text(&commit.stdout),
            },
        },
    )
}

fn branches(root: &Path, head: &Head) -> Result<Vec<BranchInfo>, WorktreeError> {
    let out = Call::new(root, Index::User)
        .args([
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            "refs/heads",
        ])
        .timeout(READ)
        .run()?;
    let current = match head {
        Head::Branch { name } => Some(name.as_str()),
        _ => None,
    };
    let mut list: Vec<BranchInfo> = String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let (full, commit) = line.split_once('\0')?;
            let name = full.strip_prefix("refs/heads/")?.to_owned();
            Some(BranchInfo {
                current: Some(name.as_str()) == current,
                name,
                commit: commit.to_owned(),
            })
        })
        .collect();
    list.sort_by(|a, b| b.current.cmp(&a.current).then_with(|| a.name.cmp(&b.name)));
    Ok(list)
}

/// Every working tree of the repository of `root` (`git worktree list --porcelain`; `-z` when git has it).
///
/// # Errors
/// git failures.
pub fn worktrees(root: &Path) -> Result<Vec<WorktreeEntry>, WorktreeError> {
    let with_z = Call::new(root, Index::User)
        .args(["worktree", "list", "--porcelain", "-z"])
        .timeout(READ)
        .output()?;
    let (printed, sep) = if with_z.ok() {
        (String::from_utf8_lossy(&with_z.stdout).into_owned(), '\0')
    } else {
        // git before 2.36 has no `-z` (exit 129); paths with a newline are then not supported.
        let plain = Call::new(root, Index::User)
            .args(["worktree", "list", "--porcelain"])
            .timeout(READ)
            .run()?;
        (String::from_utf8_lossy(&plain).into_owned(), '\n')
    };
    let mut list = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for field in printed.split(sep) {
        if let Some(path) = field.strip_prefix("worktree ") {
            list.extend(current.take());
            current = Some(WorktreeEntry {
                path: PathBuf::from(path),
                branch: None,
            });
        } else if let Some(full) = field.strip_prefix("branch ")
            && let Some(entry) = current.as_mut()
        {
            entry.branch = full.strip_prefix("refs/heads/").map(str::to_owned);
        }
    }
    list.extend(current);
    Ok(list)
}

/// Why git would run a program this repository names while a copy is made or checked (`None` when nothing
/// would). See the module docs for the rule.
///
/// # Errors
/// git failures.
pub fn preflight(root: &Path) -> Result<Option<String>, WorktreeError> {
    let out = Call::new(root, Index::User)
        .args(["config", "-z", "--list", "--show-scope"])
        .timeout(READ)
        .run()?;
    let printed = String::from_utf8_lossy(&out).into_owned();
    let mut tokens = printed.split('\0');
    while let (Some(scope), Some(entry)) = (tokens.next(), tokens.next()) {
        if !matches!(scope, "local" | "worktree") {
            continue;
        }
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        let key = key.to_ascii_lowercase();
        if let Some(reason) = program_key(&key, value) {
            return Ok(Some(format!(
                "This repository's own git settings name a program ({}) that git would run while the copy is made or checked. Vibe Work does not make copies of such repositories; work in the folder itself, or remove the setting.",
                clip(&reason, 120)
            )));
        }
    }
    Ok(None)
}

/// The setting (`key`) when it names a program git would run while a copy is made or checked.
fn program_key(key: &str, value: &str) -> Option<String> {
    if let Some(rest) = key.strip_prefix("filter.") {
        let (_, kind) = rest.rsplit_once('.')?;
        if !matches!(kind, "clean" | "smudge" | "process") {
            return None;
        }
        let canonical =
            LFS.iter().any(|(k, v)| *k == kind && value.trim() == *v) && rest.starts_with("lfs.");
        return (!canonical).then(|| key.to_owned());
    }
    if key == "core.fsmonitor" {
        let v = value.trim().to_ascii_lowercase();
        let builtin = matches!(
            v.as_str(),
            "" | "true" | "false" | "yes" | "no" | "on" | "off" | "0" | "1"
        );
        return (!builtin).then(|| key.to_owned());
    }
    None
}

/// The commit `base` (a local branch name, or a full commit id for a detached checkout) points at.
///
/// # Errors
/// `InvalidName` when it is neither a local branch nor a commit.
pub fn resolve_base(root: &Path, base: &str) -> Result<String, WorktreeError> {
    if base.is_empty() || base.starts_with('-') || base.chars().any(char::is_control) {
        return Err(WorktreeError::InvalidName(
            "the base branch name is not valid".into(),
        ));
    }
    let is_id = base.len() >= 40 && base.chars().all(|c| c.is_ascii_hexdigit());
    let spec = if is_id {
        format!("{base}^{{commit}}")
    } else {
        format!("refs/heads/{base}^{{commit}}")
    };
    let out = Call::new(root, Index::User)
        .args(["rev-parse", "--verify", "-q", &spec])
        .timeout(READ)
        .output()?;
    if out.ok() {
        Ok(text(&out.stdout))
    } else {
        Err(WorktreeError::InvalidName(format!(
            "there is no local branch {base:?} to start from"
        )))
    }
}
