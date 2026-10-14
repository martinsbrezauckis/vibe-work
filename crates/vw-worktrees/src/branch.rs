//! The session branch: checked, created and (only when untouched) deleted with the hardened runner.
//!
//! Vibe gets `agentConfig.worktree = {kind: "create", branch, name}` for a branch that already exists, so it
//! checks that branch out as it is: the base is the user's choice (Vibe has no base parameter) and Vibe never
//! fetches (it fetches `origin` only for a branch it creates itself).
//!
//! Depends on: vw-changes (`git::Call`), `error`. Called by: vw-core-api (`start`, its failure path, removal).
//! Invariants:
//! - A name is checked by `git check-ref-format --branch` and may not start with `-` before any ref is
//!   written; refs are always named in full (`refs/heads/<b>`), so no name can become an option.
//! - Creation is create-only (`update-ref` with the all-zero old value): an existing branch is never moved.
//! - Deletion happens only while the branch still points at the commit we created it at (`update-ref -d`
//!   with that old value): a branch with commits is never deleted here.
//! - `update-ref` writes one ref (and its reflog); no index, no working tree, no hook (the runner's empty
//!   hooks folder), no network.

use std::path::Path;
use std::time::Duration;

use vw_changes::git::{Call, Index};

use crate::error::{WorktreeError, clip};

const WRITE: Duration = Duration::from_secs(15);

/// Checks that `branch` can be a new branch name.
///
/// # Errors
/// `InvalidName` with git's reason in plain words.
pub fn validate(root: &Path, branch: &str) -> Result<(), WorktreeError> {
    if branch.is_empty()
        || branch.starts_with('-')
        || branch.chars().any(char::is_control)
        || branch.len() > 200
    {
        return Err(WorktreeError::InvalidName(
            "a branch name cannot be empty, start with '-', hold control characters or be longer than 200 characters".into(),
        ));
    }
    let out = Call::new(root, Index::User)
        .args(["check-ref-format", "--branch", branch])
        .timeout(WRITE)
        .output()?;
    if out.ok() && String::from_utf8_lossy(&out.stdout).trim() == branch {
        Ok(())
    } else {
        Err(WorktreeError::InvalidName(format!(
            "{branch:?} is not a valid branch name (spaces, '..', '~', '^', ':', '?', '*', '[' and a trailing '.lock' are not allowed)"
        )))
    }
}

/// True when `refs/heads/<branch>` exists.
///
/// # Errors
/// git failures.
pub fn exists(root: &Path, branch: &str) -> Result<bool, WorktreeError> {
    let out = Call::new(root, Index::User)
        .args([
            "show-ref",
            "--verify",
            "-q",
            &format!("refs/heads/{branch}"),
        ])
        .timeout(WRITE)
        .output()?;
    Ok(out.ok())
}

/// The all-zero object id of the repository's hash (`sha1`: 40, `sha256`: 64 characters).
fn zero_id(commit: &str) -> String {
    "0".repeat(commit.len())
}

/// Creates `refs/heads/<branch>` at `commit`, only if it does not exist.
///
/// # Errors
/// `InvalidName` for a bad name; `Taken` when the branch exists (nothing changed); `Git` otherwise.
pub fn create(root: &Path, branch: &str, commit: &str) -> Result<(), WorktreeError> {
    validate(root, branch)?;
    let out = Call::new(root, Index::User)
        .args([
            "update-ref",
            "-m",
            "vibe-work: branch for a session copy",
            &format!("refs/heads/{branch}"),
            commit,
            &zero_id(commit),
        ])
        .timeout(WRITE)
        .output()?;
    if out.ok() {
        return Ok(());
    }
    if exists(root, branch)? {
        return Err(WorktreeError::Taken(format!(
            "the branch {branch:?} exists already; pick another name"
        )));
    }
    Err(WorktreeError::Taken(format!(
        "could not create the branch {branch:?}: {}",
        clip(&out.stderr, 200)
    )))
}

/// Deletes `refs/heads/<branch>` only while it still points at `commit`. Returns whether it was deleted.
///
/// # Errors
/// git failures other than "the branch moved" (then `Ok(false)`).
pub fn delete_if_at(root: &Path, branch: &str, commit: &str) -> Result<bool, WorktreeError> {
    let out = Call::new(root, Index::User)
        .args(["update-ref", "-d", &format!("refs/heads/{branch}"), commit])
        .timeout(WRITE)
        .output()?;
    Ok(out.ok())
}

/// The commit `refs/heads/<branch>` points at, `None` when it does not exist.
///
/// # Errors
/// git failures.
pub fn tip(root: &Path, branch: &str) -> Result<Option<String>, WorktreeError> {
    let out = Call::new(root, Index::User)
        .args([
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{branch}^{{commit}}"),
        ])
        .timeout(WRITE)
        .output()?;
    Ok(out
        .ok()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()))
}
