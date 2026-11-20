//! Fill grants store "Always for this site" in the vault; use counts live in `secret_uses`:
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
//! Invariants: no value is ever stored or logged here; no grant decision is cached (every decision reads the vault now); any
//! read error counts as "no grant" (the dialog asks) or, right before a fill, refuses it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use vw_platform::sync::LockRecover;

use vw_keys::{FillGrant, KeyService};
use vw_vault::fill_grant::{id_for, is_grant_id};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::secrets_view::FillGrantView;

/// Use counts and last use of this run (grant id -> (uses, last use ms)).
#[derive(Default)]
pub(crate) struct FillUses(Mutex<HashMap<String, (u32, i64)>>);

/// One effect lane for each login and exact origin (the vault grant id names that pair).
/// Revoke holds the lane while changing the vault; a page writer holds it until its effect ends.
#[derive(Default)]
pub(crate) struct Effects(Mutex<HashMap<String, Arc<EffectLane>>>);

#[derive(Default)]
struct EffectLane {
    operation: Arc<tokio::sync::Mutex<()>>,
    unconfirmed: AtomicBool,
}

/// A dispatched page effect retains the lane until a confirmed result. Losing its caller marks
/// the lane unconfirmed, so a revoke can persist the change without reporting that the page is quiet.
pub(crate) struct EffectPermit {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    lane: Arc<EffectLane>,
    dispatched: bool,
    settled: bool,
}

impl EffectPermit {
    pub(crate) fn unconfirmed(&self) -> bool {
        self.lane.unconfirmed.load(Ordering::SeqCst)
    }

    pub(crate) fn dispatched(&mut self) {
        self.dispatched = true;
    }

    pub(crate) fn settled(&mut self) {
        self.settled = true;
    }
}

impl Drop for EffectPermit {
    fn drop(&mut self) {
        if self.dispatched && !self.settled {
            self.lane.unconfirmed.store(true, Ordering::SeqCst);
        }
    }
}

impl Effects {
    fn for_id(&self, id: &str) -> Arc<EffectLane> {
        let mut lanes = self.0.lock_or_recover();
        lanes
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(EffectLane::default()))
            .clone()
    }

    pub(crate) async fn enter(&self, id: &str) -> EffectPermit {
        let lane = self.for_id(id);
        let guard = lane.operation.clone().lock_owned().await;
        EffectPermit {
            _guard: guard,
            lane,
            dispatched: false,
            settled: false,
        }
    }

    fn unconfirmed(&self, id: &str) -> bool {
        self.for_id(id).unconfirmed.load(Ordering::SeqCst)
    }
}

impl FillUses {
    /// Counts one use of grant `id`, now.
    pub(crate) fn used(&self, id: &str) {
        let mut uses = self.0.lock_or_recover();
        let entry = uses.entry(id.to_owned()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = now_ms();
    }

    /// Uses of grant `id` in this run, and the last one (ms; `None` before the first).
    fn of(&self, id: &str) -> (u32, Option<i64>) {
        self.0
            .lock_or_recover()
            .get(id)
            .map_or((0, None), |(n, last)| (*n, Some(*last)))
    }
}

/// The vault's signed timestamp now (shared by the vault-facing modules; unsigned readings use `vw_changes::now_ms`).
pub(crate) use vw_platform::wall_clock::unix_ms_signed as now_ms;

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
    if shared.fill_effects.unconfirmed(&id) {
        return Err(
            "a previous browser fill has not been confirmed; close its tab and restart Vibe Work"
                .into(),
        );
    }
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

/// Revokes fill grant `id`. False when there is no such grant. An unconfirmed page effect leaves
/// the grant withdrawn but returns an error, so the caller cannot report a completed revoke.
///
/// # Errors
/// `keyService` (no vault, locked, storage).
pub(crate) async fn revoke(shared: &Shared, id: &str) -> Result<bool, CoreError> {
    let keys = keys(shared).ok_or_else(crate::keys::no_vault)?;
    if !is_grant_id(id) {
        return Ok(false);
    }
    let lane = shared.fill_effects.for_id(id);
    let _effect = lane.operation.lock().await;
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
    if after.is_some() && lane.unconfirmed.load(Ordering::SeqCst) {
        shared.emit(CoreEvent::SecretsChanged {});
        return Err(CoreError::new(
            ErrorCode::Timeout,
            "Vault access was withdrawn, but an earlier browser fill has not been confirmed to stop. Close its tab and restart Vibe Work before checking the grant again.",
        ));
    }
    Ok(after.is_some())
}

/// Revokes every active or unconfirmed grant of login `login_id` (it was removed). Returns how many.
pub(crate) async fn revoke_login(shared: &Shared, login_id: &str) -> Result<usize, CoreError> {
    let keys = keys(shared).ok_or_else(crate::keys::no_vault)?;
    let list = keys
        .fill_grants()
        .await
        .map_err(|e| crate::keys::key_error(&e))?;
    let mut n = 0;
    let mut first_error = None;
    for grant in list.into_iter().filter(|g| {
        g.login_id == login_id && (g.active() || shared.fill_effects.unconfirmed(&g.id))
    }) {
        match revoke(shared, &grant.id).await {
            Ok(true) => n += 1,
            Ok(false) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if let Some(error) = first_error {
        shared.emit(CoreEvent::SecretsChanged {});
        Err(error)
    } else {
        Ok(n)
    }
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
            let (uses, last_used_ms) = shared.secret_uses.fill_uses.of(&g.id);
            FillGrantView {
                uses,
                last_used_ms,
                id: g.id,
                login_id: g.login_id,
                login_name,
                origin: g.origin,
                granted_ms: g.granted_ms,
            }
        })
        .collect()
}
