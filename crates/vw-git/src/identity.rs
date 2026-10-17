//! Commit author settings, read and saved through the hardened runner for the user's Commit dialog.
//! Depends on vw-changes and serde; called by the core. Writes only repository-local name and email.

use crate::GitError;
use serde::{Deserialize, Serialize};
use std::path::Path;
use vw_changes::git::{Call, Index, Profile};

/// The author details the user entered in the Commit dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitIdentity {
    /// The name Git writes into the commit.
    pub name: String,
    /// The email Git writes into the commit.
    pub email: String,
}

/// Reads effective name and email independently, preserving a known field when the other is missing.
/// # Errors
/// Git failures other than a missing setting.
pub fn read(root: &Path) -> Result<(Option<String>, Option<String>), GitError> {
    Ok((
        setting(root, "user.name", false)?,
        setting(root, "user.email", false)?,
    ))
}

/// Reads only the global name and email, without writing config or using repository overrides.
/// # Errors
/// Git failures other than a missing setting.
pub fn read_global(root: &Path) -> Result<(Option<String>, Option<String>), GitError> {
    Ok((
        setting(root, "user.name", true)?,
        setting(root, "user.email", true)?,
    ))
}

fn setting(root: &Path, key: &str, global: bool) -> Result<Option<String>, GitError> {
    let mut call = Call::new(root, Index::User)
        .profile(Profile::Worktree)
        .args(["config"]);
    if global {
        call = call.args(["--global"]);
    }
    let out = call.args(["--get", key]).output()?;
    if out.code == Some(1) {
        return Ok(None);
    }
    if !out.ok() {
        return Err(GitError::Rejected(out.stderr));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok((!text.is_empty()).then_some(text))
}

/// Saves both details in this repository before its commit; validates both before writing either.
/// # Errors
/// Empty or malformed details, or a Git config write failure.
pub fn save(root: &Path, identity: &CommitIdentity) -> Result<(), GitError> {
    validate(identity)?;
    for (key, value) in [
        ("user.name", &identity.name),
        ("user.email", &identity.email),
    ] {
        Call::new(root, Index::User)
            .profile(Profile::Worktree)
            .args(["config", "--local", "--replace-all", key, value.trim()])
            .run()?;
    }
    Ok(())
}

/// Checks both author fields without writing; email format is intentionally unrestricted.
/// # Errors
/// Empty details, control characters or angle brackets in either field.
pub fn validate(identity: &CommitIdentity) -> Result<(), GitError> {
    for (label, value) in [("name", &identity.name), ("email", &identity.email)] {
        if value.trim().is_empty()
            || value
                .chars()
                .any(|c| c.is_control() || c == '<' || c == '>')
        {
            return Err(GitError::Blocked(format!(
                "Enter a {label} without line breaks or angle brackets."
            )));
        }
    }
    Ok(())
}
