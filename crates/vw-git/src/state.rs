//! The branch bar's facts: which branch the session's folder is on, an operation in progress,
//! the remotes, the upstream (as of the last fetch; this crate never fetches), and the branch against its base
//! (commits and lines). Read only: refs, config and trees; never the working tree (the uncommitted file count
//! comes from `commit::list`, which needs the program check).
//!
//! Depends on: vw-changes (the hardened runner, `discover`), `forge`, `error`. Called by: `commit`, `push`,
//! `pr`, `merge`, vw-core-api (`git` command).
//! Invariant: no call writes anything (`GIT_OPTIONAL_LOCKS=0` from the runner) and no call reaches the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vw_changes::git::{Call, Index};

use crate::error::GitError;
use crate::forge::{Forge, display_url, parse_remote};

const READ: Duration = Duration::from_secs(15);

/// What `HEAD` is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HeadState {
    /// On a branch (`name` is the short name).
    Branch {
        /// Short name.
        name: String,
    },
    /// Detached at a commit.
    Detached {
        /// The commit id.
        commit: String,
    },
    /// A branch with no commit yet.
    Unborn {
        /// Short name.
        name: String,
    },
}

/// A git operation that has stopped half-way in the folder (git would finish it on the next commit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum InProgress {
    /// `MERGE_HEAD` exists.
    Merge,
    /// A rebase stopped.
    Rebase,
    /// A cherry-pick stopped.
    CherryPick,
    /// A revert stopped.
    Revert,
}

impl InProgress {
    /// Plain words for the bar.
    #[must_use]
    pub fn words(self) -> &'static str {
        match self {
            Self::Merge => "a merge",
            Self::Rebase => "a rebase",
            Self::CherryPick => "a cherry-pick",
            Self::Revert => "a revert",
        }
    }
}

/// One remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    /// Its name (`origin`).
    pub name: String,
    /// Where a push goes, after `pushInsteadOf`/`insteadOf`, without user info.
    pub push_url: String,
    /// The forge of the fetch address (else of the push address), when it is one we know how to read.
    pub forge: Option<Forge>,
    /// The address may be pushed to (https, ssh, git, file, scp-like, a local path; never `ext::`, a
    /// `<helper>::` transport or plain http).
    pub pushable: bool,
}

/// The branch's upstream as of the last fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Upstream {
    /// The remote name.
    pub remote: String,
    /// The branch on the remote.
    pub branch: String,
    /// Commits here that the remote-tracking ref does not have.
    pub ahead: u32,
    /// Commits the remote-tracking ref has that are not here (as of the last fetch).
    pub behind: u32,
}

/// The branch against its base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BaseInfo {
    /// The base's name (a local branch, or `remote/branch` for a folder session's remote default).
    pub name: String,
    /// The base's current commit.
    pub commit: String,
    /// Commits on this branch that the base does not have.
    pub ahead: u32,
    /// Commits on the base that this branch does not have.
    pub behind: u32,
    /// Lines added on this branch since it left the base (committed work only).
    pub added: u32,
    /// Lines removed.
    pub removed: u32,
    /// Files changed.
    pub files: u32,
}

/// Everything the bar shows except the uncommitted files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BranchState {
    /// The working tree root.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub root: PathBuf,
    /// `HEAD`.
    pub head: HeadState,
    /// The commit `HEAD` points at (`None` when unborn).
    pub commit: Option<String>,
    /// An operation git stopped half-way.
    pub in_progress: Option<InProgress>,
    /// The remotes in git's order.
    pub remotes: Vec<RemoteInfo>,
    /// The upstream of the current branch.
    pub upstream: Option<Upstream>,
    /// The base the branch is compared with.
    pub base: Option<BaseInfo>,
}

impl RemoteInfo {
    /// The address for a sentence ("a remote helper address" when it is not a plain one).
    #[must_use]
    pub fn url_note(&self) -> String {
        if self.pushable {
            self.push_url.clone()
        } else {
            format!("{} (a transport Vibe Work does not use)", self.push_url)
        }
    }
}

impl BranchState {
    /// The branch name when on a branch (born or unborn).
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        match &self.head {
            HeadState::Branch { name } | HeadState::Unborn { name } => Some(name),
            HeadState::Detached { .. } => None,
        }
    }

    /// The remote a push goes to by default: the upstream's, else `origin`, else the only remote.
    #[must_use]
    pub fn default_remote(&self) -> Option<&RemoteInfo> {
        let named = |n: &str| self.remotes.iter().find(|r| r.name == n);
        self.upstream
            .as_ref()
            .and_then(|u| named(&u.remote))
            .or_else(|| named("origin"))
            .or_else(|| (self.remotes.len() == 1).then(|| &self.remotes[0]))
    }
}

/// Runs a read-only git call in `root` and returns trimmed stdout, `None` when git exits non-zero.
pub(crate) fn read(root: &Path, args: &[&str]) -> Result<Option<String>, GitError> {
    let out = Call::new(root, Index::User)
        .args(args)
        .timeout(READ)
        .output()?;
    Ok(out
        .ok()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()))
}

/// The working tree root and git dir of `folder` (`None` outside a repository).
///
/// # Errors
/// git failures (git missing, a refused folder).
pub fn locate(folder: &Path) -> Result<Option<(PathBuf, PathBuf)>, GitError> {
    match vw_changes::discover(folder) {
        Ok(repo) => Ok(repo.map(|repo| (repo.root, repo.git_dir))),
        Err(vw_changes::ChangesError::Refused(why)) => Err(GitError::Blocked(why)),
        Err(error) => Err(error.into()),
    }
}

/// Reads the branch state of the repository at `root` (git dir `git_dir`). `base` names the session copy's
/// base branch; without one, a folder session is compared with its first remote's default branch when that
/// differs from the current branch.
///
/// # Errors
/// git failures.
pub fn read_state(
    root: &Path,
    git_dir: &Path,
    base: Option<&str>,
) -> Result<BranchState, GitError> {
    let head = head(root)?;
    let commit = read(root, &["rev-parse", "--verify", "-q", "HEAD^{commit}"])?;
    let remotes = remotes(root)?;
    let upstream = match &head {
        HeadState::Branch { name } => upstream(root, name)?,
        _ => None,
    };
    let mut state = BranchState {
        root: root.to_path_buf(),
        head,
        commit,
        in_progress: in_progress(git_dir),
        remotes,
        upstream,
        base: None,
    };
    let base_ref = match base {
        Some(name) => Some((name.to_owned(), format!("refs/heads/{name}"))),
        None => remote_default(root, &state)?,
    };
    if let (Some((name, full)), Some(_)) = (base_ref, state.commit.as_ref()) {
        state.base = compare(root, &name, &full)?;
    }
    Ok(state)
}

fn head(root: &Path) -> Result<HeadState, GitError> {
    if let Some(full) = read(root, &["symbolic-ref", "-q", "HEAD"])? {
        let name = full.strip_prefix("refs/heads/").unwrap_or(&full).to_owned();
        let born = read(root, &["rev-parse", "--verify", "-q", "HEAD^{commit}"])?.is_some();
        return Ok(if born {
            HeadState::Branch { name }
        } else {
            HeadState::Unborn { name }
        });
    }
    let commit = read(root, &["rev-parse", "--verify", "-q", "HEAD^{commit}"])?.unwrap_or_default();
    Ok(HeadState::Detached { commit })
}

fn in_progress(git_dir: &Path) -> Option<InProgress> {
    if git_dir.join("MERGE_HEAD").exists() {
        Some(InProgress::Merge)
    } else if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        Some(InProgress::Rebase)
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        Some(InProgress::CherryPick)
    } else if git_dir.join("REVERT_HEAD").exists() {
        Some(InProgress::Revert)
    } else {
        None
    }
}

fn remotes(root: &Path) -> Result<Vec<RemoteInfo>, GitError> {
    let names = read(root, &["remote"])?.unwrap_or_default();
    let mut list = Vec::new();
    // A remote whose name could read as an option or a revision is left out: it is never offered.
    for name in names
        .lines()
        .map(str::trim)
        .filter(|n| crate::names::valid_remote_name(n))
    {
        let url = read(root, &["remote", "get-url", "--push", name])?.unwrap_or_default();
        let url = url.lines().next().unwrap_or("").to_owned();
        // The forge is where the repository lives (the fetch address); a push may go elsewhere (`pushurl`, a
        // fork). Pull requests and the PR card follow the fetch address.
        let fetch = read(root, &["remote", "get-url", name])?.unwrap_or_default();
        let fetch = fetch.lines().next().unwrap_or("").to_owned();
        list.push(RemoteInfo {
            name: name.to_owned(),
            forge: parse_remote(&fetch).or_else(|| parse_remote(&url)),
            pushable: crate::names::address_kind(&url) == crate::names::AddressKind::Allowed,
            push_url: display_url(&url),
        });
    }
    Ok(list)
}

fn upstream(root: &Path, branch: &str) -> Result<Option<Upstream>, GitError> {
    let remote = read(
        root,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )?;
    let merge = read(
        root,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )?;
    let (Some(remote), Some(merge)) = (remote, merge) else {
        return Ok(None);
    };
    let remote_branch = merge
        .strip_prefix("refs/heads/")
        .unwrap_or(&merge)
        .to_owned();
    if !crate::names::valid_remote_name(&remote) || !crate::names::valid_ref_name(&remote_branch) {
        return Ok(None);
    }
    let tracking = format!("refs/remotes/{remote}/{remote_branch}");
    let (ahead, behind) = if read(root, &["rev-parse", "--verify", "-q", &tracking])?.is_some() {
        counts(root, &tracking, "HEAD")?
    } else {
        // Configured but never fetched or pushed: what the remote's other refs do not have is ahead.
        let not_remote = format!("--remotes={remote}");
        let total = read(root, &["rev-list", "--count", "HEAD", "--not", &not_remote])?
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        (total, 0)
    };
    Ok(Some(Upstream {
        remote,
        branch: remote_branch,
        ahead,
        behind,
    }))
}

/// `(ahead, behind)` of `ours` against `theirs`.
pub(crate) fn counts(root: &Path, theirs: &str, ours: &str) -> Result<(u32, u32), GitError> {
    let text = read(
        root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{theirs}...{ours}"),
        ],
    )?
    .unwrap_or_default();
    let mut parts = text
        .split_whitespace()
        .map(|n| n.parse::<u32>().unwrap_or(0));
    let behind = parts.next().unwrap_or(0);
    let ahead = parts.next().unwrap_or(0);
    Ok((ahead, behind))
}

/// The first remote's default branch (`refs/remotes/<r>/HEAD`) as `(name, ref)`, when it is not the branch
/// this folder is on.
fn remote_default(root: &Path, state: &BranchState) -> Result<Option<(String, String)>, GitError> {
    let Some(remote) = state.default_remote() else {
        return Ok(None);
    };
    let Some(full) = read(
        root,
        &[
            "symbolic-ref",
            "-q",
            &format!("refs/remotes/{}/HEAD", remote.name),
        ],
    )?
    else {
        return Ok(None);
    };
    let short = full
        .strip_prefix("refs/remotes/")
        .unwrap_or(&full)
        .to_owned();
    let branch_part = short.split_once('/').map_or("", |(_, b)| b);
    if Some(branch_part) == state.branch() {
        return Ok(None);
    }
    Ok(Some((short, full)))
}

/// The default branch name on `remote` as of the last fetch (`main`), if known.
///
/// # Errors
/// git failures.
pub fn remote_default_branch(root: &Path, remote: &str) -> Result<Option<String>, GitError> {
    Ok(read(
        root,
        &[
            "symbolic-ref",
            "-q",
            "--short",
            &format!("refs/remotes/{remote}/HEAD"),
        ],
    )?
    .and_then(|s| s.split_once('/').map(|(_, b)| b.to_owned())))
}

fn compare(root: &Path, name: &str, full: &str) -> Result<Option<BaseInfo>, GitError> {
    let Some(commit) = read(
        root,
        &["rev-parse", "--verify", "-q", &format!("{full}^{{commit}}")],
    )?
    else {
        return Ok(None);
    };
    let (ahead, behind) = counts(root, &commit, "HEAD")?;
    let mut info = BaseInfo {
        name: name.to_owned(),
        commit: commit.clone(),
        ahead,
        behind,
        added: 0,
        removed: 0,
        files: 0,
    };
    let Some(fork) = read(root, &["merge-base", &commit, "HEAD"])? else {
        return Ok(Some(info));
    };
    // Plumbing diff: no textconv, no external diff, renames off (the count is per path).
    let numstat = read(
        root,
        &[
            "diff-tree",
            "-r",
            "--numstat",
            "--no-renames",
            "--no-textconv",
            "--no-ext-diff",
            &fork,
            "HEAD",
        ],
    )?
    .unwrap_or_default();
    for line in numstat.lines() {
        let mut parts = line.split('\t');
        let added = parts
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0);
        let removed = parts
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0);
        info.added = info.added.saturating_add(added);
        info.removed = info.removed.saturating_add(removed);
        info.files += 1;
    }
    Ok(Some(info))
}
