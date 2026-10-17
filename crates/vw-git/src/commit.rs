//! The user's own Commit: the exact list of files git would commit (git's own
//! `status`, with line-ending and LFS conversion), and the commit of the files the user left checked.
//!
//! git reads and writes the working tree here (the exception for the user's Commit), so every call runs
//! after [`crate::programs::check`] and with the runner's `Worktree` profile: fsmonitor off, no signing, no pager,
//! the system config (Windows line endings). The repository's commit hooks run only on the commit itself and
//! only when the per-project switch is on (like `git commit` in a terminal; default on);
//! `status` and `add` never run hooks. The commit is a pathspec commit with the
//! user's own index: entries the user staged for other files stay staged.
//!
//! Depends on: vw-changes (runner, path display), `programs`, `state`, `error`. Called by: vw-core-api.
//! Invariants:
//! - Only paths that `status` lists right now are committed; a requested path that is not listed any more
//!   makes the whole commit `Stale` (nothing is staged).
//! - Never on a detached `HEAD`, never during a merge/rebase/cherry-pick/revert (git would conclude it), never
//!   when `HEAD` moved since the dialog was built.
//! - The message and the path list go through files in our private scratch folder, never the command line.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vw_changes::git::{Call, Index, Profile};

use crate::error::{GitError, clip};
use crate::programs::{self, Purpose};
use crate::state::{self, HeadState};

/// Longest a status or commit may take (big repositories, LFS clean filters).
const SLOW: Duration = Duration::from_secs(120);

/// Longest a commit that runs the repository's hooks may take (linters, test runners).
const HOOKS_TIMEOUT: Duration = Duration::from_secs(900);

/// How a listed file changed against `HEAD` (the working tree as git sees it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CommitStatus {
    /// Changed.
    Modified,
    /// New (untracked or newly staged).
    Added,
    /// Deleted.
    Deleted,
    /// Its type changed (file ↔ symlink).
    TypeChanged,
    /// A submodule's commit changed.
    Submodule,
    /// An unresolved conflict.
    Conflicted,
}

/// One file of the commit list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CommitFile {
    /// Path relative to the repository root, `/`-separated, shown one-to-one (non-UTF-8 bytes as `\xNN`).
    pub path: String,
    /// How it changed.
    pub status: CommitStatus,
    /// The user had staged (part of) it before.
    pub staged: bool,
    /// git's raw path bytes (kept by the core, never sent).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub raw: Vec<u8>,
}

/// Lists what a commit of everything would contain (`git status`, untracked files one by one, renames as
/// delete + add, submodules by their commit only).
///
/// # Errors
/// `Blocked` from the program check; git failures.
pub fn list(root: &Path) -> Result<Vec<CommitFile>, GitError> {
    programs::check(root, Purpose::Worktree)?;
    let out = Call::new(root, Index::User)
        .profile(Profile::Worktree)
        .args([
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=dirty",
            "--no-renames",
        ])
        .timeout(SLOW)
        .run()?;
    Ok(parse_status(&out))
}

/// Parses `git status --porcelain=v2 -z --no-renames` output. Public for tests.
#[must_use]
pub fn parse_status(out: &[u8]) -> Vec<CommitFile> {
    let mut files = Vec::new();
    for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let (kind, rest) = match record.split_first() {
            Some((k, rest)) => (*k, rest.strip_prefix(b" ").unwrap_or(rest)),
            None => continue,
        };
        match kind {
            b'?' => files.push(CommitFile {
                path: vw_changes::display(rest),
                status: CommitStatus::Added,
                staged: false,
                raw: rest.to_vec(),
            }),
            b'1' | b'u' => {
                // `1 XY sub mH mI mW hH hI path` / `u XY sub m1 m2 m3 mW h1 h2 h3 path`
                let fields = if kind == b'1' { 7 } else { 9 };
                let mut parts = rest.splitn(fields + 1, |b| *b == b' ');
                let xy = parts.next().unwrap_or(b"..");
                let sub = parts.next().unwrap_or(b"N...");
                let Some(path) = parts.nth(fields - 2) else {
                    continue;
                };
                let x = xy.first().copied().unwrap_or(b'.');
                let y = xy.get(1).copied().unwrap_or(b'.');
                let status = if kind == b'u' {
                    CommitStatus::Conflicted
                } else if sub.first() == Some(&b'S') {
                    CommitStatus::Submodule
                } else if x == b'D' || y == b'D' {
                    CommitStatus::Deleted
                } else if x == b'A' {
                    CommitStatus::Added
                } else if x == b'T' || y == b'T' {
                    CommitStatus::TypeChanged
                } else {
                    CommitStatus::Modified
                };
                files.push(CommitFile {
                    path: vw_changes::display(path),
                    status,
                    staged: x != b'.',
                    raw: path.to_vec(),
                });
            }
            _ => {}
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// A short fingerprint of a list (paths and states), so a commit can tell the list moved.
#[must_use]
pub fn fingerprint(files: &[CommitFile]) -> String {
    let mut parts: Vec<Vec<u8>> = Vec::new();
    for f in files {
        parts.push(f.raw.clone());
        parts.push(format!("{:?}{}", f.status, f.staged).into_bytes());
    }
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    vw_changes::fnv_hex(&refs)
}

/// The author git would write (`git var GIT_AUTHOR_IDENT` without the date), or why there is none.
///
/// # Errors
/// `Blocked` when git has no name or email for the user.
pub fn author(root: &Path) -> Result<String, GitError> {
    let out = Call::new(root, Index::User)
        .profile(Profile::Worktree)
        .args(["var", "GIT_AUTHOR_IDENT"])
        .timeout(Duration::from_secs(10))
        .output()?;
    if !out.ok() {
        return Err(GitError::Blocked(
            "Enter your name and email in the Commit dialog. They are saved for this repository."
                .to_owned(),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // `Name <email> 1700000000 +0200`: drop the date.
    let ident = text.trim();
    let end = ident.rfind('>').map_or(ident.len(), |at| at + 1);
    Ok(ident[..end].to_owned())
}

/// The last `n` commit subjects of `HEAD` (style examples for the message), newest first.
///
/// # Errors
/// git failures.
pub fn recent_subjects(root: &Path, n: usize) -> Result<Vec<String>, GitError> {
    let count = format!("-n{n}");
    let text = state::read(root, &["log", &count, "--no-merges", "--format=%s", "HEAD"])?
        .unwrap_or_default();
    Ok(text.lines().map(str::to_owned).collect())
}

/// What the dialog showed: the commit happens only if the folder still matches it.
#[derive(Debug, Clone)]
pub struct Expected {
    /// `HEAD`'s commit when the dialog was built (`None` = unborn).
    pub head: Option<String>,
    /// The branch name.
    pub branch: String,
}

/// Commits `paths` (shown paths from [`list`]) with `message` on the current branch. `scratch` is a private
/// folder of ours for the message and path files. Returns the new commit id.
///
/// # Errors
/// `Blocked` (detached, operation in progress, program check, empty message or list), `Stale` (the branch,
/// `HEAD` or a listed path moved), `Rejected` (git refused the commit; its words), git failures.
pub fn commit(
    root: &Path,
    git_dir: &Path,
    paths: &[String],
    message: &str,
    expected: &Expected,
    scratch: &Path,
    run_hooks: bool,
) -> Result<Committed, GitError> {
    if message.trim().is_empty() {
        return Err(GitError::Blocked(
            "Write a commit message first.".to_owned(),
        ));
    }
    if paths.is_empty() {
        return Err(GitError::Blocked(
            "Choose at least one file to commit.".to_owned(),
        ));
    }
    let state = state::read_state(root, git_dir, None)?;
    match &state.head {
        HeadState::Detached { .. } => {
            return Err(GitError::Blocked(
                "This folder is not on a branch (detached HEAD). Switch to a branch first."
                    .to_owned(),
            ));
        }
        HeadState::Branch { name } | HeadState::Unborn { name } if *name != expected.branch => {
            return Err(GitError::Stale(format!(
                "The folder is on the branch {name} now, not {}. Open Commit again.",
                expected.branch
            )));
        }
        _ => {}
    }
    if let Some(op) = state.in_progress {
        return Err(GitError::Blocked(format!(
            "git has {} in progress in this folder. Finish or abort it in your git tool first.",
            op.words()
        )));
    }
    if state.commit != expected.head {
        return Err(GitError::Stale(
            "New commits arrived on this branch since the dialog opened. Open Commit again."
                .to_owned(),
        ));
    }
    let listed = list(root)?;
    let mut raw: Vec<u8> = Vec::new();
    for path in paths {
        let Some(file) = listed.iter().find(|f| &f.path == path) else {
            return Err(GitError::Stale(format!(
                "{path} has no changes to commit any more. Open Commit again."
            )));
        };
        if file.status == CommitStatus::Conflicted {
            return Err(GitError::Blocked(format!(
                "{path} has an unresolved conflict."
            )));
        }
        raw.extend_from_slice(&file.raw);
        raw.push(0);
    }
    let paths_file = scratch.join("commit-paths");
    let message_file = scratch.join("commit-message");
    write_private(&paths_file, &raw)?;
    write_private(&message_file, message.as_bytes())?;
    let result = run_commit(root, &paths_file, &message_file, run_hooks);
    let _ = std::fs::remove_file(&paths_file);
    let _ = std::fs::remove_file(&message_file);
    let output = result?;
    let id = state::read(root, &["rev-parse", "--verify", "-q", "HEAD^{commit}"])?
        .ok_or_else(|| GitError::Rejected("git made no commit".to_owned()))?;
    Ok(Committed { id, output })
}

fn run_commit(
    root: &Path,
    paths_file: &Path,
    message_file: &Path,
    run_hooks: bool,
) -> Result<String, GitError> {
    let mut spec = std::ffi::OsString::from("--pathspec-from-file=");
    spec.push(paths_file);
    let mut msg = std::ffi::OsString::from("--file=");
    msg.push(message_file);
    let add = Call::new(root, Index::User)
        .profile(Profile::Worktree)
        .args(["--literal-pathspecs", "add", "-A", "--pathspec-file-nul"])
        .args([&spec])
        .timeout(SLOW)
        .output()?;
    if !add.ok() {
        return Err(GitError::Rejected(format!(
            "git add: {}",
            clip(&add.stderr, 400)
        )));
    }
    let mut call = Call::new(root, Index::User)
        .profile(Profile::Worktree)
        .args(["--literal-pathspecs", "commit", "--pathspec-file-nul"]);
    if run_hooks {
        // The user's own commit runs the repository's commit hooks, like `git commit` in a terminal (switch on).
        call = call.with_repo_hooks().timeout(HOOKS_TIMEOUT);
    } else {
        call = call.args(["--no-verify"]).timeout(SLOW);
    }
    let commit = call.args([&msg, &spec]).output()?;
    let mut words = String::from_utf8_lossy(&commit.stdout).into_owned();
    if !commit.stderr.trim().is_empty() {
        if !words.is_empty() {
            words.push('\n');
        }
        words.push_str(&commit.stderr);
    }
    if commit.ok() {
        Ok(clip(&words, 8000))
    } else {
        Err(GitError::Rejected(format!(
            "The commit did not happen. git commit said:\n{}",
            clip(&words, 4000)
        )))
    }
}

/// A finished commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// The new commit id.
    pub id: String,
    /// What git and the hooks printed.
    pub output: String,
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), GitError> {
    std::fs::write(path, bytes).map_err(|e| GitError::io("write", path, e))
}
