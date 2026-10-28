//! Captured project authority for vault policies; Git metadata never changes an active session's identity.
//!
//! Depends on: `vw-platform`, `vw-worktrees`, `vw-keys`, `core::Shared`. Called by: session opening,
//! `secrets`, and the secret-run/login-fill consumers. Current User only on the default or either project candidate wins.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::core::Shared;
use vw_keys::{EntryPermission, KeyError, KeyService};

/// Canonical folder and optional main candidate, with inheritance authorized only at capture time.
#[derive(Clone, Debug)]
pub(crate) struct Identity {
    pub(crate) plain: String,
    main: Option<String>,
    inherit: bool,
}

/// All candidate records, retained so even a change that leaves the effective policy unchanged refuses a use.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Policy {
    pub(crate) permission: EntryPermission,
    records: Vec<Option<(EntryPermission, i64)>>,
}

impl Identity {
    /// The only project whose Allowed record can authorize this identity.
    pub(crate) fn allowed_project(&self) -> &str {
        if self.inherit {
            self.main.as_deref().unwrap_or(&self.plain)
        } else {
            &self.plain
        }
    }

    /// Reads both candidates, refusing errors and taking User only before any Allowed or site grant.
    pub(crate) async fn policy(&self, keys: &KeyService, id: &str) -> Result<Policy, KeyError> {
        let current_default = keys.default_entry_permission(id).await?;
        let default = current_default.0;
        let plain = keys.own_entry_permission(&self.plain, id).await?;
        let mut records = vec![Some(current_default), plain];
        let selected = if let Some(main) = &self.main {
            let other = keys.own_entry_permission(main, id).await?;
            records.push(other);
            if self.inherit {
                other.map_or(default, |p| p.0)
            } else {
                plain.map_or(default, |p| p.0)
            }
        } else {
            plain.map_or(default, |p| p.0)
        };
        Ok(Policy {
            permission: if records
                .iter()
                .flatten()
                .any(|(p, _)| *p == EntryPermission::Never)
            {
                EntryPermission::Never
            } else {
                selected
            },
            records,
        })
    }

    /// Stores the person's Always answer without overwriting a concurrent project choice.
    pub(crate) async fn allow(
        &self,
        keys: &KeyService,
        id: &str,
        policy: &Policy,
    ) -> Result<Policy, String> {
        if self.policy(keys, id).await.ok().as_ref() != Some(policy) {
            return Err(
                "Not run: an entry permission changed meanwhile; ask the user to try again"
                    .to_owned(),
            );
        }
        let selected = if self.inherit { 2 } else { 1 };
        let stored = keys
            .allow_entry_if_unchanged(
                self.allowed_project().to_owned(),
                id.to_owned(),
                policy.records[selected],
            )
            .await
            .map_err(|_| {
                "Not run: the project permission changed or could not be stored".to_owned()
            })?;
        let mut records = policy.records.clone();
        records[selected] = Some(stored);
        Ok(Policy {
            permission: EntryPermission::Allowed,
            records,
        })
    }
}

/// Canonical folders the user picked or that the core already knows as project/session folders.
pub(crate) fn registered(shared: &Shared) -> BTreeSet<PathBuf> {
    let mut paths = shared.gate.picked_roots();
    paths.extend(shared.projects.store.read(|s| {
        s.snapshot()
            .projects
            .iter()
            .filter_map(|p| p.main_folder.as_ref().map(PathBuf::from))
            .collect::<Vec<_>>()
    }));
    paths.extend(
        shared
            .names
            .session_folders(&shared.store)
            .into_iter()
            .filter_map(|(id, p)| {
                shared.entry_projects.known(&id).map_or_else(
                    || Some(PathBuf::from(p)),
                    |identity| identity.ok().map(|i| PathBuf::from(i.plain)),
                )
            }),
    );
    paths.extend(shared.entry_projects.folders());
    paths
        .into_iter()
        .filter_map(|p| vw_platform::canonical_plain(&p).ok())
        .collect()
}

/// Captures both policy candidates. Git's claim alone never grants main-project authority.
pub(crate) fn capture(folder: &str, registered: &BTreeSet<PathBuf>) -> Result<Identity, String> {
    let plain = vw_platform::canonical_plain(Path::new(folder))
        .map_err(|_| "the project folder could not be resolved".to_owned())?;
    if !plain.is_dir() {
        return Err("the project folder could not be resolved".to_owned());
    }
    let main = vw_worktrees::repo_info::read(&plain)
        .map_err(|_| "the project's worktree identity could not be checked".to_owned())?
        .map(|info| vw_platform::canonical_plain(info.main_root()))
        .transpose()
        .map_err(|_| "the project's main folder could not be resolved".to_owned())?
        .filter(|main| *main != plain);
    let inherit = main
        .as_ref()
        .is_some_and(|main| registered.contains(main) && linked_both_ways(&plain, main));
    Ok(Identity {
        plain: plain.to_string_lossy().into_owned(),
        main: main.map(|p| p.to_string_lossy().into_owned()),
        inherit,
    })
}

/// Reads only a regular, small Git link file, never a symlink or arbitrary Git-selected file.
fn link_text(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > 8192 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn resolve_link(parent: &Path, text: &str) -> Option<PathBuf> {
    let path = Path::new(text.trim());
    vw_platform::canonical_plain(&parent.join(path)).ok()
}

/// The admin entry must be exactly main/.git/worktrees/<name>, and its gitdir must point back here.
fn linked_both_ways(plain: &Path, main: &Path) -> bool {
    let marker = plain.join(".git");
    let Some(text) = link_text(&marker) else {
        return false;
    };
    let Some(forward) = text.trim().strip_prefix("gitdir: ") else {
        return false;
    };
    let Some(admin) = resolve_link(plain, forward) else {
        return false;
    };
    let Some(name) = admin.file_name() else {
        return false;
    };
    let expected = main.join(".git").join("worktrees").join(name);
    // Do not accept an alternate admin tree through symlinks, nor read a backlink outside this layout.
    if admin != expected || !admin.is_dir() {
        return false;
    }
    let Some(back) = link_text(&admin.join("gitdir")) else {
        return false;
    };
    let Some(common) = link_text(&admin.join("commondir")) else {
        return false;
    };
    resolve_link(&admin, &back) == vw_platform::canonical_plain(&marker).ok()
        && resolve_link(&admin, &common) == vw_platform::canonical_plain(&main.join(".git")).ok()
}

/// User-only folder commands inspect current verified linkage; running consumers use their session snapshot.
pub(crate) fn user_identity(
    shared: &Shared,
    folder: &str,
) -> Result<Identity, crate::error::CoreError> {
    capture(folder, &registered(shared)).map_err(|reason| {
        crate::error::CoreError::new(crate::error::ErrorCode::InvalidParams, reason)
    })
}

/// The policy write target for a user-only command.
pub(crate) fn user_project(
    shared: &Shared,
    folder: &str,
) -> Result<String, crate::error::CoreError> {
    Ok(user_identity(shared, folder)?.allowed_project().to_owned())
}
