//! Fill grants mirror `secret_grants` for "Always for this site":
//!
//! - **Once:** never stored: the approval covers exactly the one fill the dialog showed.
//! - **Always for this site:** a hidden vault record per (login, exact page origin) (`vw_vault::fill_grant`,
//!   `userOnly`), read from the vault for every decision and changed atomically (`KeyService::update_fill_grant`).
//! - **Revoke** (Settings > Project secrets, `secrets revoke`, removing the login): `always` off and `revokedMs` set on
//!   the vault's current record; the record stays so a dialog open during the revoke is refused afterwards.
//! - Use counts: memory only, since Vibe Work started.
//!
//! Depends on: vw-keys (`fill_grant`, `fill_grants`, `update_fill_grant`), vw-vault (`fill_grant`), `secrets_view`.
//! Called by: `browser_fill` (find, grant, check), `secrets` (list, revoke, login removal).
//! Invariants: no value is ever stored or logged here; nothing is cached (every decision reads the vault now); any
//! read error counts as "no grant" (the dialog asks) or, right before a fill, refuses it.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use vw_keys::{FillGrant, KeyService};
use vw_vault::fill_grant::{id_for, is_grant_id};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::secrets_view::FillGrantView;

/// Use counts of this run (grant id -> uses).
#[derive(Default)]
pub(crate) struct FillUses(Mutex<HashMap<String, u32>>);

impl FillUses {
    /// Counts one use of grant `id`.
    pub(crate) fn used(&self, id: &str) {
        let mut uses = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let n = uses.entry(id.to_owned()).or_insert(0);
        *n = n.saturating_add(1);
    }

    fn of(&self, id: &str) -> u32 {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .copied()
            .unwrap_or(0)
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn keys(shared: &Shared) -> Option<&KeyService> {
    shared.config.keys.as_deref()
}

/// The record of login `login_id` on `origin` as the vault holds it now. `Err`: it could not be read.
async fn current(
    shared: &Shared,
    login_id: &str,
    origin: &str,
) -> Result<Option<FillGrant>, String> {
    let keys = keys(shared).ok_or("no vault is connected")?;
    let record = keys
        .fill_grant(id_for(login_id, origin))
        .await
        .map_err(|e| e.to_string())?;
    // The id is derived from both and `from_item` checks it; compared again so a decision never rests on the id alone.
    Ok(record.filter(|g| g.login_id == login_id && g.origin.eq_ignore_ascii_case(origin)))
}

/// The id of the active "always" grant of this login on this origin, read now; a read error counts as none.
pub(crate) async fn find(shared: &Shared, login_id: &str, origin: &str) -> Option<String> {
    match current(shared, login_id, origin).await {
        Ok(record) => record.filter(FillGrant::active).map(|g| g.id),
        Err(error) => {
            tracing::warn!(%error, "fill grants: not readable now, so the user is asked");
            None
        }
    }
}

/// True while the grant still holds (checked right before the password is read: a revoke in between wins; a read
/// error refuses).
pub(crate) async fn holds(shared: &Shared, login_id: &str, origin: &str) -> bool {
    matches!(current(shared, login_id, origin).await, Ok(Some(g)) if g.active())
}

/// True when this login's grant on this origin was revoked at or after `asked`, or the vault cannot say (fail
/// closed). Used after a plain "Allow once".
pub(crate) async fn revoked_during(
    shared: &Shared,
    login_id: &str,
    origin: &str,
    asked: i64,
) -> bool {
    match current(shared, login_id, origin).await {
        Ok(record) => record.is_some_and(|g| g.revoked_ms >= asked),
        Err(_) => true,
    }
}

/// Records the user's "Always for this site" (dialog opened at `asked`) atomically; a record revoked while the dialog
/// was open is left as it is and the answer refused. Returns the grant id, or the reason (no value).
pub(crate) async fn grant_always(
    shared: &Shared,
    login_id: &str,
    login_name: &str,
    origin: &str,
    asked: i64,
) -> Result<String, String> {
    let keys = keys(shared).ok_or("no vault is connected")?;
    let id = id_for(login_id, origin);
    let answered = now_ms();
    let (record_id, login, name, site) = (
        id.clone(),
        login_id.to_owned(),
        login_name.to_owned(),
        origin.to_owned(),
    );
    let after = keys
        .update_fill_grant(id.clone(), move |current| {
            let mut grant = match current {
                Some(existing) if existing.revoked_ms >= asked => return None,
                Some(existing) if existing.active() => return None,
                Some(existing) => existing,
                None => FillGrant {
                    id: record_id.clone(),
                    login_id: login.clone(),
                    login_name: String::new(),
                    origin: site.clone(),
                    always: false,
                    granted_ms: 0,
                    revoked_ms: 0,
                    created_ms: answered,
                },
            };
            grant.login_name.clone_from(&name);
            grant.always = true;
            grant.granted_ms = answered.max(grant.revoked_ms.saturating_add(1));
            Some(grant)
        })
        .await
        .map_err(|e| e.to_string())?;
    match after {
        Some(g) if g.revoked_ms >= asked => {
            Err("the permission was revoked while the dialog was open; ask again".to_owned())
        }
        Some(g) if g.active() => Ok(id),
        _ => Err("the permission could not be stored in the vault".to_owned()),
    }
}

/// Revokes fill grant `id`. False when there is no such grant.
///
/// # Errors
/// `keyService` (no vault, locked, storage).
pub(crate) async fn revoke(shared: &Shared, id: &str) -> Result<bool, CoreError> {
    let keys = keys(shared).ok_or_else(|| {
        let mut error = CoreError::new(ErrorCode::KeyService, "no vault is connected");
        error.server_code = Some("notConfigured".to_owned());
        error
    })?;
    if !is_grant_id(id) {
        return Ok(false);
    }
    let now = now_ms();
    let after = keys
        .update_fill_grant(id.to_owned(), move |current| {
            let mut grant = current?;
            grant.always = false;
            grant.revoked_ms = grant.revoked_ms.max(now).max(grant.granted_ms);
            Some(grant)
        })
        .await
        .map_err(|e| crate::keys::key_error(&e))?;
    Ok(after.is_some())
}

/// Revokes every active grant of login `login_id` (it was removed). Returns how many.
pub(crate) async fn revoke_login(shared: &Shared, login_id: &str) -> usize {
    let Some(keys) = keys(shared) else {
        return 0;
    };
    let Ok(list) = keys.fill_grants().await else {
        return 0;
    };
    let mut n = 0;
    for grant in list
        .into_iter()
        .filter(|g| g.active() && g.login_id == login_id)
    {
        if matches!(revoke(shared, &grant.id).await, Ok(true)) {
            n += 1;
        }
    }
    n
}

/// The active grants for Settings > Project secrets (newest first). Empty while the vault cannot be read.
pub(crate) async fn views(shared: &Shared) -> Vec<FillGrantView> {
    let Some(keys) = keys(shared) else {
        return Vec::new();
    };
    let Ok(list) = keys.fill_grants().await else {
        return Vec::new();
    };
    let names = keys.logins().await.ok().unwrap_or_default();
    list.into_iter()
        .filter(FillGrant::active)
        .map(|g| {
            let login_name = names
                .iter()
                .find(|l| l.id == g.login_id)
                .map_or(g.login_name, |l| l.name.clone());
            FillGrantView {
                uses: shared.secret_grants.fill_uses.of(&g.id),
                id: g.id,
                login_id: g.login_id,
                login_name,
                origin: g.origin,
                granted_ms: g.granted_ms,
            }
        })
        .collect()
}
