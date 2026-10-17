//! Repository program checks: before git reads or writes the working tree for the user's own
//! Commit or "Bring into main", or reaches the network for a Push, the repository's own config must not make
//! git run a program or send the user's credentials somewhere else.
//!
//! Only the repository's own scopes count (`local`, `worktree`; included files are reported under the scope
//! that includes them): the system and global config are the user's own and stay in force (credential
//! helpers, ssh, line endings).
//! - **Worktree** (status, add, commit, fast-forward, merge-tree): the worktree preflight
//!   (`vw_worktrees::preflight`: filter and fsmonitor programs; the canonical git-lfs filter is allowed), plus
//!   merge drivers (they run in `merge-tree`) and include directives. Hooks, the editor, the pager, external
//!   diff, textconv and signing never run: the hardened runner pins them on every call.
//! - **Publish** (push): an **allowlist**. Every repository-scope key in a Git or companion namespace must be one of [`PUBLISH_SAFE`] (data git
//!   needs: the repository format, line endings, remotes' addresses and refspecs, branch tracking, the user's
//!   name). Unknown custom namespaces are inert metadata: no supported push transport or hook consumes them.
//!   Git's runtime config inventory also restricts namespaces added by a newer executable. Remote addresses must be https, ssh, git, file, scp-like or a
//!   local path (`names::address_kind`).
//!
//! Depends on: vw-changes (the hardened runner), vw-worktrees (worktree preflight), `names`. Called by: `commit`,
//! `push`, `merge`, vw-core-api.
//! Invariant: read only (`git config --list --show-scope`).

use std::path::Path;
use std::time::Duration;

use vw_changes::git::{Call, Index};

use crate::error::{GitError, clip};
use crate::names::{AddressKind, address_kind};

mod namespaces;

/// What git is about to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Read and write the working tree (status, add, commit, fast-forward, merge-tree).
    Worktree,
    /// Reach the remote with the user's credentials (push). Includes everything of `Worktree`.
    Publish,
}

/// Repository-scope keys a push accepts, as `section.name` or `section.*.name` (a `*` matches any
/// subsection). Everything here is data; none of it names a program, a proxy, a transport or a credential.
pub const PUBLISH_SAFE: &[&str] = &[
    "core.repositoryformatversion",
    "core.filemode",
    "core.bare",
    "core.logallrefupdates",
    "core.ignorecase",
    "core.precomposeunicode",
    "core.symlinks",
    "core.autocrlf",
    "core.eol",
    "core.safecrlf",
    "core.longpaths",
    "core.sparsecheckout",
    "core.sparsecheckoutcone",
    "core.untrackedcache",
    "core.splitindex",
    "core.checkstat",
    "core.trustctime",
    "core.quotepath",
    "core.commitgraph",
    "core.multipackindex",
    "core.abbrev",
    "core.fsmonitor",
    "extensions.objectformat",
    "extensions.refstorage",
    "extensions.worktreeconfig",
    "extensions.partialclone",
    "extensions.preciousobjects",
    "remote.*.url",
    "remote.*.pushurl",
    "remote.*.fetch",
    "remote.*.tagopt",
    "remote.*.prune",
    "remote.*.prunetags",
    "remote.*.promisor",
    "remote.*.partialclonefilter",
    "remote.*.skipdefaultupdate",
    "remote.*.skipfetchall",
    "branch.*.remote",
    "branch.*.merge",
    "branch.*.rebase",
    "branch.*.description",
    "branch.*.pushremote",
    "user.name",
    "user.email",
    "init.defaultbranch",
    "pull.rebase",
    "pull.ff",
    "push.default",
    "push.autosetupremote",
    "fetch.prune",
    "gc.auto",
    "lfs.repositoryformatversion",
    "status.showuntrackedfiles",
    "protocol.version",
    "http.postbuffer",
];

/// Checks the repository at `root`. `Ok(())` when its own config is fine for `purpose`.
///
/// # Errors
/// `Blocked` naming the first setting that is not; git failures.
pub fn check(root: &Path, purpose: Purpose) -> Result<(), GitError> {
    if let Some(reason) =
        vw_worktrees::preflight(root).map_err(|e| GitError::Blocked(e.to_string()))?
    {
        // The worktree preflight message talks about copies; ours names the same setting for this action.
        return Err(GitError::Blocked(reason.replace(
            "Vibe Work does not make copies of such repositories; work in the folder itself, or remove the setting.",
            "Vibe Work does not run it; use your own git tool for this repository, or remove the setting.",
        )));
    }
    let out = Call::new(root, Index::User)
        .args(["config", "-z", "--list", "--show-scope"])
        .timeout(Duration::from_secs(10))
        .run()?;
    let printed = String::from_utf8_lossy(&out).into_owned();
    // The executable's inventory keeps newly introduced Git namespaces restricted after an upgrade.
    let inventory = if purpose == Purpose::Publish {
        let out = Call::new(root, Index::User)
            .args(["help", "--config"])
            .timeout(Duration::from_secs(10))
            .run()?;
        String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|line| {
                line.split_once('.')
                    .map(|(section, _)| section.to_ascii_lowercase())
            })
            .collect()
    } else {
        Vec::new()
    };
    if let Some(key) = first_program_key_with_inventory(&printed, purpose, &inventory) {
        return Err(GitError::Blocked(format!(
            "This repository's own git settings include {} , which Vibe Work does not accept for this action \
             (it could run a program or send your sign-in elsewhere). Use your own git tool for this repository, \
             or remove the setting.",
            clip(&key, 120)
        )));
    }
    Ok(())
}

/// The first repository-scope key of `printed` (`git config -z --list --show-scope`) refused for `purpose`.
/// Public for tests.
#[must_use]
pub fn first_program_key(printed: &str, purpose: Purpose) -> Option<String> {
    first_program_key_with_inventory(printed, purpose, &[])
}

fn first_program_key_with_inventory(
    printed: &str,
    purpose: Purpose,
    inventory: &[String],
) -> Option<String> {
    let mut tokens = printed.split('\0');
    while let (Some(scope), Some(entry)) = (tokens.next(), tokens.next()) {
        // `command` = the runner's own `-c` pins; `global`/`system` = the user's own config.
        if !matches!(scope, "local" | "worktree") {
            continue;
        }
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        let key = key.to_ascii_lowercase();
        if refused(&key, value, purpose, inventory) {
            return Some(key);
        }
    }
    None
}

/// `(section, has a subsection, name)` of a lower-case key.
fn parts(key: &str) -> (&str, bool, &str) {
    let section = key.split('.').next().unwrap_or("");
    let name = key.rsplit('.').next().unwrap_or("");
    (section, key.matches('.').count() >= 2, name)
}

fn allowed_for_publish(key: &str) -> bool {
    let (section, sub, name) = parts(key);
    PUBLISH_SAFE.iter().any(|pattern| {
        let (p_section, p_sub, p_name) = parts(pattern);
        p_section == section && p_sub == sub && p_name == name
    })
}

fn refused(key: &str, value: &str, purpose: Purpose, inventory: &[String]) -> bool {
    let (section, sub, name) = parts(key);
    if section == "include" || section == "includeif" {
        return true;
    }
    // Merge drivers run during `merge-tree` (Bring into main).
    if section == "merge" && sub && name == "driver" {
        return true;
    }
    if purpose == Purpose::Worktree {
        return false;
    }
    if section == "filter" && sub {
        // The canonical git-lfs filter passed the worktree preflight; it is data for a push.
        return !key.starts_with("filter.lfs.");
    }
    if !allowed_for_publish(key) {
        return !key.contains('.') || !namespaces::custom(section, inventory);
    }
    section == "remote"
        && matches!(name, "url" | "pushurl")
        && address_kind(value) == AddressKind::Refused
}
