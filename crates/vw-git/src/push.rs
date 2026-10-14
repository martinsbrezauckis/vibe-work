//! The user's own Push: the network operation initiated by the user. The core asks the
//! native confirmation (naming the remote, its address and the branch) before it calls [`push`].
//!
//! Credentials are the user's own: the runner's `Publish` profile keeps the system and global config
//! (credential helpers, `core.sshCommand`, Git Credential Manager, osxkeychain) and the ssh agent; the
//! repository's own config may not name any program for it ([`crate::programs::check`] with `Publish`), and
//! `--receive-pack` is pinned so a repository cannot pick the program a local or file remote runs. We never
//! store a token. Hooks do not run (no `pre-push`, so Git LFS objects are not uploaded: [`uses_lfs`] lets the
//! dialog say so first). Never a force push.
//!
//! Depends on: vw-changes (runner), `programs`, `state`, `forge`, `error`. Called by: vw-core-api.
//! Invariants: the branch is pushed under its own name to the named remote; the upstream is set when the
//! branch has none; the push sends the confirmed commit id itself (`<sha>:refs/heads/<branch>`), and only while
//! the branch, its commit and the remote's address are still what the user confirmed.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vw_changes::git::{Call, Index, Profile};

use crate::error::{GitError, clip};
use crate::names;
use crate::programs::{self, Purpose};
use crate::state::{self, BranchState};

/// Longest a push may take (big first pushes over slow links).
const PUSH_TIMEOUT: Duration = Duration::from_secs(600);

/// What a push will do, for the confirmation and the dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PushPlan {
    /// The remote's name.
    pub remote: String,
    /// Where it goes (no user info).
    pub url: String,
    /// The local branch.
    pub branch: String,
    /// The branch name on the remote.
    pub remote_branch: String,
    /// The commit that will be pushed.
    pub commit: String,
    /// Commits the remote does not have (by its remote-tracking refs, as of the last fetch).
    pub commits: u32,
    /// The first push of this branch (the upstream will be set).
    pub first_push: bool,
    /// The repository uses Git LFS (its objects are not uploaded by this push).
    pub lfs: bool,
    /// The exact push address git gets as its target (`url` is this without user info; kept by the core, never
    /// sent: it may hold a token).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub target: String,
}

/// Builds the push plan for `remote` (default: the upstream's remote, `origin`, or the only remote).
///
/// # Errors
/// `Blocked` (no branch, no commit, no remote, program check), `NotFound` (unknown remote), git failures.
pub fn plan(root: &Path, state: &BranchState, remote: Option<&str>) -> Result<PushPlan, GitError> {
    let Some(branch) = state.branch() else {
        return Err(GitError::Blocked(
            "This folder is not on a branch (detached HEAD), so there is nothing to push."
                .to_owned(),
        ));
    };
    let Some(commit) = state.commit.clone() else {
        return Err(GitError::Blocked(
            "This branch has no commit yet. Commit first.".to_owned(),
        ));
    };
    let target = match remote {
        Some(name) => state
            .remotes
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| GitError::NotFound(format!("There is no remote called {name}.")))?,
        None => state.default_remote().ok_or_else(|| {
            GitError::Blocked(if state.remotes.is_empty() {
                "This repository has no remote to push to. Add one in your git tool (git remote add origin <address>)."
                    .to_owned()
            } else {
                "This repository has several remotes and none is called origin: choose one.".to_owned()
            })
        })?,
    };
    programs::check(root, Purpose::Publish)?;
    if !target.pushable {
        return Err(GitError::Blocked(format!(
            "Vibe Work pushes only to https, ssh, git and local addresses; {} points to {}.",
            target.name,
            target.url_note()
        )));
    }
    let addresses = state::read(
        root,
        &["remote", "get-url", "--push", "--all", &target.name],
    )?
    .unwrap_or_default();
    let mut lines = addresses.lines();
    let push_target = lines.next().unwrap_or("").to_owned();
    if lines.next().is_some() {
        return Err(GitError::Blocked(format!(
            "The remote {} pushes to several addresses; push it from your own git tool.",
            target.name
        )));
    }
    let upstream = state.upstream.as_ref().filter(|u| u.remote == target.name);
    let remote_branch = upstream.map_or_else(|| branch.to_owned(), |u| u.branch.clone());
    for name in [branch, remote_branch.as_str()] {
        if !names::valid_ref_name(name) {
            return Err(GitError::Blocked(format!(
                "The branch name {name:?} cannot be pushed from Vibe Work (it could be read as an option or a \
                 revision). Rename the branch in your git tool."
            )));
        }
    }
    let tracking = format!("refs/remotes/{}/{remote_branch}", target.name);
    let commits = if state::read(root, &["rev-parse", "--verify", "-q", &tracking])?.is_some() {
        state::counts(root, &tracking, "HEAD")?.0
    } else {
        // A new branch: the commits the remote does not have yet (as far as its remote-tracking refs say).
        let remotes = format!("--remotes={}", target.name);
        state::read(root, &["rev-list", "--count", "HEAD", "--not", &remotes])?
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    Ok(PushPlan {
        remote: target.name.clone(),
        url: target.push_url.clone(),
        branch: branch.to_owned(),
        remote_branch,
        commit,
        commits,
        first_push: upstream.is_none(),
        lfs: uses_lfs(root)?,
        target: push_target,
    })
}

/// True when the repository's `.gitattributes` at `HEAD` sends files through the LFS filter.
///
/// # Errors
/// git failures.
pub fn uses_lfs(root: &Path) -> Result<bool, GitError> {
    let attributes =
        state::read(root, &["cat-file", "-p", "HEAD:.gitattributes"])?.unwrap_or_default();
    Ok(attributes.contains("filter=lfs"))
}

/// The plain sentence for a confirmation: what leaves the machine, where to.
#[must_use]
pub fn describe(plan: &PushPlan) -> String {
    let commits = match plan.commits {
        0 => "no new commits".to_owned(),
        1 => "1 commit".to_owned(),
        n => format!("{n} commits"),
    };
    let lfs = if plan.lfs {
        " This repository uses Git LFS: large files are not uploaded by this push (run git lfs push in your own git tool)."
    } else {
        ""
    };
    format!(
        "Push the branch {} ({commits}) to {} ({}) as {}?{lfs}",
        plan.branch, plan.remote, plan.url, plan.remote_branch
    )
}

/// Pushes as planned, after checking that `HEAD` is still `plan.commit`. Returns git's summary line.
///
/// # Errors
/// `Stale` (`HEAD` moved), `Blocked` (program check), `Rejected` (the remote or the credentials said no:
/// git's words, with a plain sentence for a non-fast-forward), git failures.
pub fn push(root: &Path, plan: &PushPlan) -> Result<String, GitError> {
    // Exactly what was confirmed (trap 7): the same branch checked out, at the same commit, to the same
    // address; anything else is refused, nothing is pushed.
    let changed = || {
        GitError::Stale(
            "This changed since you confirmed the push — review again (press Push again)."
                .to_owned(),
        )
    };
    let head = state::read(root, &["rev-parse", "--verify", "-q", "HEAD^{commit}"])?;
    let current = state::read(root, &["symbolic-ref", "-q", "--short", "HEAD"])?;
    let tip = state::read(
        root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{}^{{commit}}", plan.branch),
        ],
    )?;
    if head.as_deref() != Some(plan.commit.as_str())
        || tip.as_deref() != Some(plan.commit.as_str())
        || current.as_deref() != Some(plan.branch.as_str())
    {
        return Err(changed());
    }
    programs::check(root, Purpose::Publish)?;
    // Checked again right before git runs: the names, and the address git will really use.
    if !names::valid_remote_name(&plan.remote)
        || !names::valid_ref_name(&plan.branch)
        || !names::valid_ref_name(&plan.remote_branch)
    {
        return Err(GitError::Blocked(
            "The remote or branch name cannot be used.".to_owned(),
        ));
    }
    // The address now must be the one confirmed; git then gets that exact string as the target (no second
    // resolution), and git's own protocol allowlist (`GIT_ALLOW_PROTOCOL`, set by the Publish profile) decides
    // the transport. Our classification only words the dialog (parser differential).
    let address = state::read(
        root,
        &["remote", "get-url", "--push", "--all", &plan.remote],
    )?
    .unwrap_or_default();
    if address != plan.target || plan.target.is_empty() || plan.target.starts_with('-') {
        return Err(changed());
    }
    // The confirmed commit itself, never a branch name that could move after the dialog.
    if !(plan.commit.len() >= 40 && plan.commit.chars().all(|c| c.is_ascii_hexdigit())) {
        return Err(changed());
    }
    let refspec = format!("{}:refs/heads/{}", plan.commit, plan.remote_branch);
    let out = Call::new(root, Index::User)
        .profile(Profile::Publish)
        .args([
            "push",
            "--porcelain",
            "--receive-pack=git-receive-pack",
            "--no-verify",
        ])
        // `--` first: nothing after it can be read as an option.
        .args(["--", plan.target.as_str(), refspec.as_str()])
        .timeout(PUSH_TIMEOUT)
        .output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.ok() {
        // Pushed to an address, git updates no remote-tracking ref: record what the remote has now, as a push
        // to the remote's name would, and the upstream on a first push (what `--set-upstream` writes).
        let tracking = format!("refs/remotes/{}/{}", plan.remote, plan.remote_branch);
        Call::new(root, Index::User)
            .args([
                "update-ref",
                "-m",
                "vibe-work: push",
                &tracking,
                &plan.commit,
            ])
            .timeout(std::time::Duration::from_secs(10))
            .run()?;
        if plan.first_push {
            set_upstream(root, plan)?;
        }
        return Ok(format!(
            "Pushed {} to {} ({}).",
            plan.branch, plan.remote, plan.url
        ));
    }
    Err(GitError::Rejected(rejection(&stdout, &out.stderr)))
}

/// Records the upstream of `plan.branch` as `plan.remote`/`plan.remote_branch` (`branch.<b>.remote/merge`).
fn set_upstream(root: &Path, plan: &PushPlan) -> Result<(), GitError> {
    let merge = format!("refs/heads/{}", plan.remote_branch);
    for (key, value) in [
        (
            format!("branch.{}.remote", plan.branch),
            plan.remote.as_str(),
        ),
        (format!("branch.{}.merge", plan.branch), merge.as_str()),
    ] {
        Call::new(root, Index::User)
            .args(["config", "--local", "--", key.as_str(), value])
            .timeout(std::time::Duration::from_secs(10))
            .run()?;
    }
    Ok(())
}

/// Plain words for a failed push. Public for tests.
#[must_use]
pub fn rejection(stdout: &str, stderr: &str) -> String {
    let both = format!("{stdout}\n{stderr}");
    if both.contains("[rejected]")
        && (both.contains("non-fast-forward") || both.contains("fetch first"))
    {
        return "The remote branch has commits this branch does not have, so git did not push (Vibe Work never \
                force-pushes). Bring those commits in first: ask Vibe to update the copy from the remote, or use \
                your own git tool."
            .to_owned();
    }
    let lower = both.to_ascii_lowercase();
    if lower.contains("authentication failed")
        || lower.contains("permission denied")
        || lower.contains("could not read username")
        || lower.contains("terminal prompts disabled")
    {
        return format!(
            "The remote did not accept your credentials. Vibe Work uses your own git sign-in (credential manager or \
             ssh key); sign in once with your own git tool, then press Push again. git said: {}",
            clip(stderr, 300)
        );
    }
    format!(
        "git push failed: {}",
        clip(
            if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            },
            400
        )
    )
}
