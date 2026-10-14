//! The secret broker's grants and use log (moved the grants into the vault).
//!
//! - **Once:** never stored: the approval covers exactly the one request the dialog showed, and is logged.
//! - **Always for this command:** a hidden vault record per (project, pinned command fingerprint)
//!   (`vw_vault::command_grant`, `userOnly`), read from the vault for every decision and changed atomically
//!   (`KeyService::update_command_grant`). Before the grants were a plain JSON file in the agent tools' data folder;
//!   the agent's commands run as the same user and could plant an "always" grant for a command of their own there.
//!   A vault record needs the vault's unlock material, and the vault's head guard (`vw_vault::chain`) refuses an older
//!   vault version brought back by copying files.
//! - **Revoke** (Settings > Project secrets, the Debug API acting as the user, removal of a secret): `always` off and
//!   `revokedMs` set on the vault's current record, so every window and app that shares the vault sees it at once.
//!   The record stays, so a dialog that was open during the revoke is refused afterwards (`grant_always`,
//!   `revoked_during`).
//! - **Use counts and the use log: memory only, since Vibe Work started.** A vault write commits a whole new sealed
//!   manifest, so writing them per run would grow the user's vault (and its sync) by one or two versions per
//!   `secret_run`; neither is a security decision. They hold no value.
//!
//! Depends on: vw-keys (`command_grants`, `command_grant`, `update_command_grant`), vw-vault (`command_grant`),
//! `secrets_view` (the row types). Called by: `agent_tools/secret_run` (find, grant, check, use, log), `secrets`
//! (list, revoke, secret removal), `core` (open).
//! Invariants:
//! - No value is ever stored or logged here: ids, names, variable names, programs, folders, times.
//! - Nothing about a grant is cached: every decision reads the vault now; any read error counts as "no grant" (fail
//!   closed: the dialog asks, or a run about to start is refused).
//! - The locks are never held across an await.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use vw_keys::{CommandGrant, KeyService};
use vw_vault::command_grant::{id_for, same_project};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::secrets_view::{GrantView, SecretUseView};

/// Log entries kept.
pub(crate) const MAX_LOG: usize = 200;
/// The legacy grant file, used only by development builds before grant storage moved to the vault.
const LEGACY_FILE: &str = "secret-grants.json";

/// Use counts and the use log of this run (module docs).
#[derive(Default)]
pub(crate) struct SecretGrants {
    /// Grant id -> (uses, last use ms) since Vibe Work started.
    uses: Mutex<HashMap<String, (u32, i64)>>,
    log: Mutex<VecDeque<SecretUseView>>,
    /// The fill grants' use counts of this run (kept here so the core's state stays in one place).
    pub(crate) fill_uses: crate::fill_grants::FillUses,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

impl SecretGrants {
    /// The store, after deleting the legacy grant file in the agent tools' data folder `dir` unread: an agent may
    /// have planted entries in it, so dev-build users approve again on the next use. Nothing reads that file any more,
    /// so one written later is inert. Exit condition: remove this cleanup after 0.2.
    pub(crate) fn open(dir: Option<&Path>) -> Self {
        if let Some(path) = dir.map(|d| d.join(LEGACY_FILE)) {
            match std::fs::remove_file(&path) {
                Ok(()) => tracing::info!(
                    "the old secret grant file was removed unread; grants live in the vault now"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(%error, "the old secret grant file could not be removed");
                }
            }
        }
        Self::default()
    }

    /// Counts one use of grant `id` in this run.
    pub(crate) fn used(&self, id: &str) {
        let mut uses = self.uses.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = uses.entry(id.to_owned()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = now_ms();
    }

    fn uses_of(&self, id: &str) -> (u32, Option<i64>) {
        self.uses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .map_or((0, None), |(n, at)| (*n, Some(*at)))
    }

    /// Appends one log entry (newest first; the oldest past [`MAX_LOG`] is dropped).
    pub(crate) fn log(&self, entry: SecretUseView) {
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.push_front(entry);
        log.truncate(MAX_LOG);
    }

    /// The log of this run, newest first.
    pub(crate) fn log_entries(&self) -> Vec<SecretUseView> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }
}

fn keys(shared: &Shared) -> Option<&KeyService> {
    shared.config.keys.as_deref()
}

/// The record of `fingerprint` in `project` as the vault holds it now. `Err` = it could not be read (locked vault,
/// storage, a vault version the head guard refused); `Ok(None)` = no valid record.
async fn current(
    shared: &Shared,
    project: &str,
    fingerprint: &str,
) -> Result<Option<CommandGrant>, String> {
    let keys = keys(shared).ok_or("no vault is connected")?;
    let record = keys
        .command_grant(id_for(project, fingerprint))
        .await
        .map_err(|e| e.to_string())?;
    // The id is derived from both, and `from_item` checks that; compared again so a decision never rests on the id.
    Ok(record.filter(|g| g.fingerprint == fingerprint && same_project(&g.project, project)))
}

/// The id of the active "always" grant of exactly this pinned command in `project`, read from the vault now. Any read
/// error counts as no grant (logged; the dialog then asks, as for a first use).
pub(crate) async fn find(shared: &Shared, fingerprint: &str, project: &str) -> Option<String> {
    match current(shared, project, fingerprint).await {
        Ok(record) => record.filter(CommandGrant::active).map(|g| g.id),
        Err(error) => {
            tracing::warn!(%error, "secret grants: not readable now, so the user is asked");
            None
        }
    }
}

/// True while the grant of this command is still active in the vault (checked right before a granted run starts:
/// a revoke in between wins; a read error refuses).
pub(crate) async fn holds(shared: &Shared, fingerprint: &str, project: &str) -> bool {
    matches!(current(shared, project, fingerprint).await, Ok(Some(g)) if g.active())
}

/// True when this command's record was revoked at or after `asked` (while its dialog was open), or the vault cannot
/// say (fail closed). Used after a plain "Allow once".
pub(crate) async fn revoked_during(
    shared: &Shared,
    fingerprint: &str,
    project: &str,
    asked: i64,
) -> bool {
    match current(shared, project, fingerprint).await {
        Ok(record) => record.is_some_and(|g| g.revoked_ms >= asked),
        Err(_) => true,
    }
}

/// Why an "always" answer was not stored.
pub(crate) enum NotGranted {
    /// The command's grant was revoked while the dialog was open (its `revokedMs` is at or after `asked`).
    RevokedMeanwhile,
    /// The vault could not store it (the reason, no value).
    Vault(String),
}

/// What the dialog showed, for the record (display fields and the secrets).
pub(crate) struct NewGrant {
    pub(crate) project: String,
    pub(crate) fingerprint: String,
    pub(crate) command: String,
    pub(crate) bindings: Vec<String>,
    pub(crate) secret_ids: Vec<String>,
}

/// Records the user's "always for this command" (dialog opened at `asked`, answered at `answered`) atomically on the
/// vault's current record: a record revoked while the dialog was open is left as it is and the answer refused
/// Returns the grant id.
pub(crate) async fn grant_always(
    shared: &Shared,
    new: NewGrant,
    asked: i64,
    answered: i64,
) -> Result<String, NotGranted> {
    let keys = keys(shared).ok_or_else(|| NotGranted::Vault("no vault is connected".to_owned()))?;
    let id = id_for(&new.project, &new.fingerprint);
    let record_id = id.clone();
    let after = keys
        .update_command_grant(id.clone(), move |current| {
            let mut grant = match current {
                Some(existing) if existing.revoked_ms >= asked => return None,
                Some(existing) if existing.active() => return None,
                Some(existing) => existing,
                None => CommandGrant {
                    id: record_id.clone(),
                    project: new.project.clone(),
                    fingerprint: new.fingerprint.clone(),
                    command: String::new(),
                    bindings: Vec::new(),
                    secret_ids: Vec::new(),
                    always: false,
                    granted_ms: 0,
                    revoked_ms: 0,
                    created_ms: answered,
                },
            };
            grant.command.clone_from(&new.command);
            grant.bindings.clone_from(&new.bindings);
            grant.secret_ids.clone_from(&new.secret_ids);
            grant.always = true;
            // After every earlier revoke, so the grant is active (`revokedMs < asked <= answered`).
            grant.granted_ms = answered.max(grant.revoked_ms.saturating_add(1));
            Some(grant)
        })
        .await
        .map_err(|e| NotGranted::Vault(e.to_string()))?;
    match after {
        Some(g) if g.revoked_ms >= asked => Err(NotGranted::RevokedMeanwhile),
        Some(g) if g.active() => Ok(id),
        _ => Err(NotGranted::Vault(
            "the grant record could not be written".to_owned(),
        )),
    }
}

/// Revokes grant `id`: `always` off, `revokedMs` now, on the vault's current record. False when there is no such
/// grant.
///
/// # Errors
/// `keyService` (no vault, locked vault: `vaultLocked`, storage).
pub(crate) async fn revoke(shared: &Shared, id: &str) -> Result<bool, CoreError> {
    let keys = keys(shared).ok_or_else(|| {
        let mut error = CoreError::new(ErrorCode::KeyService, "no vault is connected");
        error.server_code = Some("notConfigured".to_owned());
        error
    })?;
    if !vw_vault::command_grant::is_grant_id(id) {
        return Ok(false);
    }
    let now = now_ms();
    let after = keys
        .update_command_grant(id.to_owned(), move |current| {
            let mut grant = current?;
            grant.always = false;
            grant.revoked_ms = grant.revoked_ms.max(now).max(grant.granted_ms);
            Some(grant)
        })
        .await
        .map_err(|e| crate::keys::key_error(&e))?;
    Ok(after.is_some())
}

/// Revokes every active grant of secret `secret_id` (it was removed). Returns how many. A vault that cannot be read
/// revokes nothing (logged): the removed secret cannot be read either, so no grant of it can inject anything.
pub(crate) async fn revoke_secret(shared: &Shared, secret_id: &str) -> usize {
    let Some(keys) = keys(shared) else {
        return 0;
    };
    let list = match keys.command_grants().await {
        Ok(list) => list,
        Err(error) => {
            tracing::warn!(%error, "secret grants: not read while removing a secret");
            return 0;
        }
    };
    let mut n = 0;
    for grant in list
        .into_iter()
        .filter(|g| g.active() && g.secret_ids.iter().any(|s| s == secret_id))
    {
        match revoke(shared, &grant.id).await {
            Ok(true) => n += 1,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "secret grants: a grant of a removed secret was not revoked");
            }
        }
    }
    n
}

/// The active grants for Settings > Project secrets and `secret_list` (newest first), with this run's use counts.
/// Empty while the vault cannot be read.
pub(crate) async fn views(shared: &Shared) -> Vec<GrantView> {
    let Some(keys) = keys(shared) else {
        return Vec::new();
    };
    let Ok(list) = keys.command_grants().await else {
        return Vec::new();
    };
    let names = keys.project_secrets().await.ok().unwrap_or_default();
    list.into_iter()
        .filter(CommandGrant::active)
        .map(|g| {
            let (uses, last_used_ms) = shared.secret_grants.uses_of(&g.id);
            let mut bindings = g.bindings;
            for (binding, id) in bindings.iter_mut().zip(&g.secret_ids) {
                if let Some(secret) = names.iter().find(|s| &s.id == id)
                    && let Some((variable, _)) = binding.split_once(" <- ")
                {
                    *binding = format!("{variable} <- {}", secret.name);
                }
            }
            GrantView {
                id: g.id,
                command: g.command,
                bindings,
                secret_ids: g.secret_ids,
                project: g.project,
                fingerprint: g.fingerprint,
                created_ms: g.granted_ms,
                uses,
                last_used_ms,
            }
        })
        .collect()
}
