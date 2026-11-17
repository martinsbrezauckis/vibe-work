//! Pull request head identity from a checkout's branch configuration, including gh's URL-valued fork remotes.
//! Depends on the hardened read-only git runner through `state`; called by the gh card lookup.

use std::path::Path;

use crate::error::GitError;
use crate::forge::{Forge, parse_remote};
use crate::{names, state};

/// The remote head of a local branch, qualified by its owner when its tracking repository is a fork.
/// A checkout of a pull-request ref identifies the pull request by number instead.
///
/// # Errors
/// Read-only git failures.
pub fn head_for_branch(root: &Path, forge: &Forge, branch: &str) -> Result<String, GitError> {
    let merge = state::read(
        root,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )?;
    let Some(merge) = merge else {
        return Ok(branch.to_owned());
    };
    if let Some(number) = merge
        .strip_prefix("refs/pull/")
        .and_then(|s| s.strip_suffix("/head"))
        && number.parse::<u64>().is_ok_and(|n| n > 0)
    {
        return Ok(number.to_owned());
    }
    let Some(head) = merge
        .strip_prefix("refs/heads/")
        .filter(|h| names::valid_ref_name(h))
    else {
        return Ok(branch.to_owned());
    };
    let remote = state::read(
        root,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )?;
    let Some(remote) = remote else {
        return Ok(branch.to_owned());
    };
    let url = if names::valid_remote_name(&remote) {
        state::read(root, &["remote", "get-url", &remote])?.unwrap_or_default()
    } else {
        remote
    };
    let Some(tracking) = parse_remote(&url).filter(|f| f.host == forge.host) else {
        return Ok(branch.to_owned());
    };
    Ok(if tracking.path.eq_ignore_ascii_case(&forge.path) {
        head.to_owned()
    } else {
        let owner = tracking.path.split('/').next().unwrap_or_default();
        format!("{owner}:{head}")
    })
}

#[cfg(test)]
#[path = "gh_head_tests.rs"]
mod tests;
