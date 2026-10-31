//! Commit author settings, read and saved through the hardened runner for the user's Commit dialog.
//! Depends on vw-changes and serde; called by the core. Writes only repository-local name and email, and never from a
//! linked worktree (a copy shares its repository's config with the main checkout and every other copy).

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

/// Where the person's own (global) Git settings are: Git's `--global` file, or an explicit file (tests).
#[derive(Clone, Copy, Debug)]
pub enum Global<'a> {
    /// Git's own global config (`~/.gitconfig` or the XDG one, as Git resolves it).
    Git,
    /// This file instead (a test home's config).
    File(&'a Path),
}

/// The name and email Git resolves for a commit in `root` as the agent's shell runs it (system config off): the
/// repository's own (when `root` is in one), else `global`'s.
/// # Errors
/// Git failures other than a missing setting.
pub fn effective(
    root: &Path,
    global: Global<'_>,
) -> Result<(Option<String>, Option<String>), GitError> {
    let one = |key: &str| -> Result<Option<String>, GitError> {
        // Outside a repository `--local` fails: no repository value.
        if let Ok(Some(value)) = scoped(root, key, &["--local"]) {
            return Ok(Some(value));
        }
        global_setting(root, key, global)
    };
    Ok((one("user.name")?, one("user.email")?))
}

/// Saves `identity` in `global`, only for a field that has no value there yet (an existing global value is never
/// overwritten). Validates first.
/// # Errors
/// Empty or malformed details, or a Git config failure.
pub fn save_global(
    root: &Path,
    global: Global<'_>,
    identity: &CommitIdentity,
) -> Result<(), GitError> {
    validate(identity)?;
    for (key, value) in [
        ("user.name", &identity.name),
        ("user.email", &identity.email),
    ] {
        if global_setting(root, key, global)?.is_some() {
            continue;
        }
        let file;
        let place: Vec<&std::ffi::OsStr> = match global {
            Global::Git => vec![std::ffi::OsStr::new("--global")],
            Global::File(path) => {
                file = path.as_os_str();
                vec![std::ffi::OsStr::new("--file"), file]
            }
        };
        Call::new(root, Index::User)
            .profile(Profile::Worktree)
            .args(["config"])
            .args(place)
            .args([key, value.trim()])
            .run()?;
    }
    Ok(())
}

fn global_setting(root: &Path, key: &str, global: Global<'_>) -> Result<Option<String>, GitError> {
    match global {
        Global::Git => setting(root, key, true),
        Global::File(path) => {
            if !path.exists() {
                return Ok(None);
            }
            let mut args = vec![
                std::ffi::OsString::from("--file"),
                path.as_os_str().to_owned(),
            ];
            args.push("--get".into());
            args.push(key.into());
            let out = Call::new(root, Index::User)
                .profile(Profile::Worktree)
                .args(["config"])
                .args(args)
                .output()?;
            config_value(&out)
        }
    }
}

fn scoped(root: &Path, key: &str, scope: &[&str]) -> Result<Option<String>, GitError> {
    let out = Call::new(root, Index::User)
        .profile(Profile::Worktree)
        .args(["config"])
        .args(scope)
        .args(["--get", key])
        .output()?;
    config_value(&out)
}

fn config_value(out: &vw_changes::git::Output) -> Result<Option<String>, GitError> {
    if out.code == Some(1) {
        return Ok(None);
    }
    if !out.ok() {
        return Err(GitError::Rejected(out.stderr.clone()));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok((!text.is_empty()).then_some(text))
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

/// True when `root` is a linked worktree: its repository-local config is the main checkout's, shared with every copy.
/// # Errors
/// Git failures.
pub fn shares_config(root: &Path) -> Result<bool, GitError> {
    let out = Call::new(root, Index::User)
        .profile(Profile::Worktree)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ])
        .output()?;
    if !out.ok() {
        return Err(GitError::Rejected(out.stderr));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines().map(|line| {
        let path = Path::new(line.trim());
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    });
    Ok(match (lines.next(), lines.next()) {
        (Some(dir), Some(common)) => dir != common,
        _ => false,
    })
}

/// Saves both details in this repository before its commit; validates both before writing either.
/// # Errors
/// Empty or malformed details, a linked worktree ([`shares_config`]: refused, nothing written), or a Git config write
/// failure.
pub fn save(root: &Path, identity: &CommitIdentity) -> Result<(), GitError> {
    validate(identity)?;
    if shares_config(root)? {
        return Err(GitError::Blocked(
            "This copy shares its repository's settings; the name and email are not saved there."
                .to_owned(),
        ));
    }
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
