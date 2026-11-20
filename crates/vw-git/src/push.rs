//! User Push and app-mediated agent push. The core confirms the destination according to the caller's policy
//! before executing the bound plan. User Push uses [`push`]; agent pushes use [`push_local_branch`].
//!
//! Credentials are the user's own: the runner's `Publish` profile keeps the system and global config
//! (credential helpers, `core.sshCommand`, Git Credential Manager, osxkeychain) and the ssh agent; the
//! repository's own config may not name any program for it ([`crate::programs::check`] with `Publish`), and
//! `--receive-pack` is pinned so a repository cannot pick the program a local or file remote runs. We never
//! store a token. Client hooks do not run, so Git LFS objects are not uploaded. Agent local receivers also use
//! a credential-free runner, checked repository config and a fixed receiver with hooks disabled. Network server
//! hooks remain the server's behavior. Never a force push.
//!
//! Depends on: vw-changes (runner), `programs`, `state`, `forge`, `error`. Called by: vw-core-api.
//! Invariants: an agent branch is pushed under its own name; User Push may follow a configured upstream alias.
//! A missing upstream is set. The push sends the confirmed commit id (`<sha>:refs/heads/<branch>`) only while the
//! local tip and remote address still match the plan. User Push also requires that branch to remain checked out.

use std::fmt::Write as _;
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

#[cfg(unix)]
const LOCAL_RECEIVE_PACK: &str =
    "git -c core.hooksPath=/dev/null -c gc.auto=0 -c maintenance.auto=false receive-pack";
#[cfg(windows)]
const LOCAL_RECEIVE_PACK: &str =
    "git -c core.hooksPath=NUL -c gc.auto=0 -c maintenance.auto=false receive-pack";

/// What a push will do, for the confirmation and the dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PushPlan {
    /// The remote's name.
    pub remote: String,
    /// The exact address Git resolves for this push (credentials and fragments are refused).
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
    /// The resolved push address, retained by the core to check the destination again before execution.
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
    let push_target = resolved_target(root, &target.name)?;
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
        url: push_target.clone(),
        branch: branch.to_owned(),
        remote_branch,
        commit,
        commits,
        first_push: upstream.is_none(),
        lfs: uses_lfs(root)?,
        target: push_target,
    })
}

/// Plans an app-owned push of a local branch under its own name. A named branch need not be
/// checked out. With no remote, its upstream remote wins, then the Push button's usual default.
///
/// # Errors
/// Missing local branch or remote, unsafe repository programs, or a failed Git read.
pub fn plan_local_branch(
    root: &Path,
    state: &BranchState,
    remote: Option<&str>,
    branch: Option<&str>,
) -> Result<PushPlan, GitError> {
    let branch = branch
        .or_else(|| state.branch())
        .ok_or_else(|| GitError::Blocked("Choose a local branch to push.".into()))?;
    if !names::valid_ref_name(branch) {
        return Err(GitError::Blocked(
            "The local branch name cannot be used.".into(),
        ));
    }
    let commit = state::read(
        root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{branch}^{{commit}}"),
        ],
    )?
    .ok_or_else(|| GitError::NotFound(format!("There is no committed local branch {branch}.")))?;
    let upstream_remote = state::read(
        root,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )?;
    let upstream_merge = state::read(
        root,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )?;
    let upstream = upstream_remote
        .as_deref()
        .zip(upstream_merge.as_deref())
        .filter(|(name, merge)| {
            names::valid_remote_name(name)
                && merge
                    .strip_prefix("refs/heads/")
                    .is_some_and(names::valid_ref_name)
        });
    let target = match remote {
        Some(name) => state
            .remotes
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| GitError::NotFound(format!("There is no remote called {name}.")))?,
        None => upstream
            .and_then(|(name, _)| state.remotes.iter().find(|r| r.name == name))
            .or_else(|| state.default_remote())
            .ok_or_else(|| {
                GitError::Blocked(if state.remotes.is_empty() {
                    "This repository has no remote to push to. Add one in your git tool (git remote add origin <address>).".into()
                } else {
                    "This repository has several remotes and none is called origin: choose one.".into()
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
    let push_target = resolved_target(root, &target.name)?;
    let tracking = format!("refs/remotes/{}/{branch}", target.name);
    let commits = if state::read(root, &["rev-parse", "--verify", "-q", &tracking])?.is_some() {
        state::counts(root, &tracking, &commit)?.0
    } else {
        let remotes = format!("--remotes={}", target.name);
        state::read(root, &["rev-list", "--count", &commit, "--not", &remotes])?
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    Ok(PushPlan {
        remote: target.name.clone(),
        url: push_target.clone(),
        branch: branch.to_owned(),
        remote_branch: branch.to_owned(),
        commit,
        commits,
        first_push: upstream.is_none_or(|(name, _)| name != target.name.as_str()),
        lfs: uses_lfs_at(root, &format!("refs/heads/{branch}"))?,
        target: push_target,
    })
}

/// Git resolves pushInsteadOf/insteadOf once, using the Publish configuration just as `push` does.
fn resolved_target(root: &Path, remote: &str) -> Result<String, GitError> {
    let bytes = Call::new(root, Index::User)
        .profile(Profile::Publish)
        .args(["remote", "get-url", "--push", "--all", remote])
        .timeout(Duration::from_secs(20))
        .run()?;
    let addresses = String::from_utf8(bytes)
        .map_err(|_| GitError::Blocked("The push address is not valid UTF-8.".into()))?;
    // Remove only Git's output delimiter, never whitespace belonging to the actual URL.
    let address = addresses.strip_suffix('\n').unwrap_or(&addresses);
    if address.is_empty() || address.starts_with('-') || address.contains(['\n', '\r']) {
        return Err(GitError::Blocked(format!(
            "The remote {remote} must have one push address."
        )));
    }
    // URL parsing must recognize # and ? as authority terminators, never as part of a host/user name.
    // SSH's login name (including scp-style git@host:path) is not a password or token.
    if address.contains("://") {
        let url = url::Url::parse(address)
            .map_err(|_| GitError::Blocked("The push address is not a valid URL.".into()))?;
        if url.fragment().is_some()
            || url.password().is_some()
            || (!url.username().is_empty() && url.scheme() != "ssh")
        {
            return Err(GitError::Blocked(
                "The push address must not contain credentials or a fragment.".into(),
            ));
        }
    }
    Ok(address.to_owned())
}

/// A local push address needs no Git login. Its receiver runs on this computer, so its own
/// repository configuration must pass the same program check before `receive-pack` starts.
fn local_destination(root: &Path, address: &str) -> Result<Option<std::path::PathBuf>, GitError> {
    let path = if address.starts_with("file://") {
        let url = url::Url::parse(address)
            .map_err(|_| GitError::Blocked("The local push address is invalid.".into()))?;
        if url.query().is_some() {
            return Err(GitError::Blocked(
                "The local push address must not contain a query.".into(),
            ));
        }
        url.to_file_path()
            .map_err(|()| GitError::Blocked("The file push address is not a local path.".into()))?
    } else if address.contains("://") {
        return Ok(None);
    } else {
        let path = Path::new(address);
        #[cfg(windows)]
        if address.as_bytes().get(1) == Some(&b':') && !path.is_absolute() {
            return Err(GitError::Blocked(
                "The local push address must be an absolute drive path.".into(),
            ));
        }
        // Git treats `host:path` as scp-style only when no path separator precedes `:`.
        // Thus `nested/remote:bare` and explicit relative paths stay local.
        let scp_style = address.find(':').is_some_and(|colon| {
            !(address[..colon].contains('/') || cfg!(windows) && address[..colon].contains('\\'))
        });
        if !path.is_absolute() && scp_style {
            return Ok(None);
        }
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };
    std::fs::canonicalize(&path).map(Some).map_err(|error| {
        GitError::Blocked(format!("The local push address cannot be opened: {error}."))
    })
}

/// True when the repository's `.gitattributes` at `HEAD` sends files through the LFS filter.
///
/// # Errors
/// git failures.
pub fn uses_lfs(root: &Path) -> Result<bool, GitError> {
    uses_lfs_at(root, "HEAD")
}

fn uses_lfs_at(root: &Path, revision: &str) -> Result<bool, GitError> {
    let attributes = state::read(
        root,
        &["cat-file", "-p", &format!("{revision}:.gitattributes")],
    )?
    .unwrap_or_default();
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
    push_bound(root, plan, true)
}

/// Pushes a previously planned local branch without checking it out. The remote ref must retain
/// the local branch's own name, and the branch tip and destination are checked again.
///
/// # Errors
/// A stale plan, unsafe repository setting, or a rejected Git push.
pub fn push_local_branch(root: &Path, plan: &PushPlan) -> Result<String, GitError> {
    if plan.remote_branch != plan.branch {
        return Err(GitError::Blocked(
            "An agent push must use the local branch's own name on the remote.".into(),
        ));
    }
    push_bound(root, plan, false)
}

fn push_bound(root: &Path, plan: &PushPlan, require_head: bool) -> Result<String, GitError> {
    // The confirmed local tip and remote address must still match. The user's Push also binds HEAD.
    let changed = || {
        GitError::Stale(if require_head {
            "This changed since you confirmed the push. Review it again (press Push again).".into()
        } else {
            "This branch or remote changed since the push was approved. Review it again.".into()
        })
    };
    let head = if require_head {
        state::read(root, &["rev-parse", "--verify", "-q", "HEAD^{commit}"])?
    } else {
        None
    };
    let current = if require_head {
        state::read(root, &["symbolic-ref", "-q", "--short", "HEAD"])?
    } else {
        None
    };
    let tip = state::read(
        root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{}^{{commit}}", plan.branch),
        ],
    )?;
    if tip.as_deref() != Some(plan.commit.as_str())
        || require_head
            && (head.as_deref() != Some(plan.commit.as_str())
                || current.as_deref() != Some(plan.branch.as_str()))
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
    // Resolve with the same config and push direction as execution, then use the remote name.
    // Passing the expanded URL back to Git would expand insteadOf a second time.
    if resolved_target(root, &plan.remote)? != plan.target || plan.url != plan.target {
        return Err(changed());
    }
    // The confirmed commit itself, never a branch name that could move after the dialog.
    if !(plan.commit.len() >= 40 && plan.commit.chars().all(|c| c.is_ascii_hexdigit())) {
        return Err(changed());
    }
    let local = if require_head {
        None
    } else {
        local_destination(root, &plan.target)?
    };
    if let Some(destination) = &local {
        programs::check(destination, Purpose::Publish)?;
    }
    let refspec = format!("{}:refs/heads/{}", plan.commit, plan.remote_branch);
    let call = Call::new(root, Index::User);
    let call = if local.is_some() {
        // A local receiver can execute in this OS account. It receives no Git login environment;
        // its own hooks are disabled by the fixed receive-pack command, even if its config names them.
        call.profile(Profile::Worktree)
            .args(["-c", "protocol.file.allow=always"])
            .args([
                "push",
                "--porcelain",
                &format!("--receive-pack={LOCAL_RECEIVE_PACK}"),
                "--no-verify",
                "--no-follow-tags",
                "--recurse-submodules=no",
            ])
    } else {
        call.profile(Profile::Publish).args([
            "push",
            "--porcelain",
            "--receive-pack=git-receive-pack",
            "--no-verify",
            // Only the confirmed ref leaves: a repository's or the person's push.followTags / push.recurseSubmodules
            // must not add tags or submodule pushes the card never named.
            "--no-follow-tags",
            "--recurse-submodules=no",
        ])
    };
    let out = call
        // `--` first: nothing after it can be read as an option.
        .args(["--", plan.remote.as_str(), refspec.as_str()])
        .timeout(PUSH_TIMEOUT)
        .output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.ok() {
        return Ok(record_pushed(root, plan));
    }
    Err(GitError::Rejected(rejection(&stdout, &out.stderr)))
}

/// Records a confirmed remote success; local bookkeeping failures remain explicit warnings.
fn record_pushed(root: &Path, plan: &PushPlan) -> String {
    let tracking = format!("refs/remotes/{}/{}", plan.remote, plan.remote_branch);
    let tracking_result = Call::new(root, Index::User)
        .args([
            "update-ref",
            "-m",
            "vibe-work: push",
            &tracking,
            &plan.commit,
        ])
        .timeout(Duration::from_secs(10))
        .run();
    let upstream_result = if plan.first_push {
        set_upstream(root, plan)
    } else {
        Ok(())
    };
    let mut result = format!("Pushed {} to {} ({}).", plan.branch, plan.remote, plan.url);
    if let Err(error) = tracking_result {
        let _ = write!(
            result,
            " The push succeeded, but the local tracking ref was not updated: {error}."
        );
    }
    if let Err(error) = upstream_result {
        let _ = write!(
            result,
            " The push succeeded, but the local upstream was not set: {error}."
        );
    }
    result
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
        return "The server has changes this branch does not have (non-fast-forward), so nothing was pushed (Vibe Work never \
                overwrites the server). Get those changes first: ask Vibe to update this branch from the server, or use \
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
