//! Website logins and their fill grants on the key service: list the vault's logins (metadata), store or
//! remove one Vibe Work created, hand a login's password to the core's browser fill, and read or change the "always
//! for this site" grants (`vw_vault::fill_grant`).
//!
//! Depends on: vw-vault (`login`, `fill_grant`, the vault), `service`. Called by: vw-core-api (`logins`,
//! `fill_grants`, `browser_fill`).
//! Invariants:
//! - Lists and results carry metadata only ([`Login`], [`FillGrant`]). The only function that returns a password is
//!   [`KeyService::login_password`], for the core's fill, which writes it into one page field; it is registered as a
//!   stored secret before it is returned, and every call is counted ([`Counters`]).
//! - Every read goes to the vault now (nothing is cached), so a revoke by another window or app is seen at once; a
//!   locked vault answers [`KeyError::Vault`] `Locked`.

use std::sync::atomic::{AtomicU64, Ordering};

use vw_vault::SecretString;
use vw_vault::fill_grant::{self, FillGrant};
use vw_vault::login::{self, Login, NewLogin};

use crate::error::KeyError;
use crate::service::{KeyService, blocking, now_ms};

/// How often the logins API touched the vault (diagnostics for tests: proves a refused fill never reads a
/// password; never values).
#[derive(Default)]
pub(crate) struct Counters {
    lookups: AtomicU64,
    password_reads: AtomicU64,
}

/// A snapshot of [`Counters`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoginCounts {
    /// Vault reads of the logins (list or one login): each decrypts whole login items in memory.
    pub lookups: u64,
    /// Passwords handed out ([`KeyService::login_password`]).
    pub password_reads: u64,
}

/// How a store call went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLogin {
    /// The login (metadata).
    pub login: Login,
    /// True when an own login with the same site and user name was replaced.
    pub replaced: bool,
}

impl KeyService {
    /// Renames an own login while keeping its id, site, username, password and grants.
    ///
    /// # Errors
    /// Invalid name, missing or foreign login, or vault failure.
    pub async fn rename_login(&self, id: String, name: String) -> Result<(), KeyError> {
        if name.trim().is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control)
        {
            return Err(KeyError::InvalidSecret(
                "the login name must be 1 to 80 characters of text",
            ));
        }
        let vault = self.vault();
        blocking(move || {
            let current = vault.read_item(&id)?.ok_or(KeyError::NoSuchSecret)?;
            if !login::from_item(&current).is_some_and(|l| l.own) {
                return Err(KeyError::NotOwnSecret);
            }
            vault
                .update_item(&id, |current| {
                    let mut item = current?;
                    if !login::from_item(&item).is_some_and(|l| l.own) {
                        return None;
                    }
                    name.trim().clone_into(&mut item.title);
                    item.updated_ms = now_ms();
                    Some(item)
                })?
                .ok_or(KeyError::NoSuchSecret)?;
            Ok(())
        })
        .await?
    }

    /// Replaces an own login's password, preserving its exact bytes and all other fields.
    ///
    /// # Errors
    /// Invalid password, missing or foreign login, or vault failure.
    pub async fn replace_login_password(
        &self,
        id: String,
        password: SecretString,
    ) -> Result<(), KeyError> {
        if !(login::MIN_PASSWORD..=login::MAX_PASSWORD).contains(&password.len()) {
            return Err(KeyError::InvalidSecret(
                "the password must be 8 to 4096 bytes (Vibe Work can hide only passwords of 8 or more)",
            ));
        }
        let vault = self.vault();
        blocking(move || {
            let current = vault.read_item(&id)?.ok_or(KeyError::NoSuchSecret)?;
            if !login::from_item(&current).is_some_and(|l| l.own) {
                return Err(KeyError::NotOwnSecret);
            }
            vault
                .update_item(&id, |current| {
                    let mut item = current?;
                    if !login::from_item(&item).is_some_and(|l| l.own) {
                        return None;
                    }
                    password.expose().clone_into(&mut item.password);
                    item.updated_ms = now_ms();
                    Some(item)
                })?
                .ok_or(KeyError::NoSuchSecret)?;
            password.register_as(vw_secrets::SecretKind::VaultValue);
            Ok(())
        })
        .await?
    }

    /// Reads a listed login's user name for a user-initiated clipboard operation.
    ///
    /// # Errors
    /// Missing or non-login item, or vault failure.
    pub async fn listed_login_username(&self, id: String) -> Result<String, KeyError> {
        let vault = self.vault();
        blocking(move || {
            let item = vault.read_item(&id)?.ok_or(KeyError::NoSuchSecret)?;
            login::from_item(&item)
                .map(|l| l.username)
                .ok_or(KeyError::NoSuchSecret)
        })
        .await?
    }

    /// Reads a listed login's password for a user-initiated clipboard operation.
    ///
    /// # Errors
    /// Missing or unusable login, or vault failure.
    pub async fn listed_login_password(&self, id: String) -> Result<SecretString, KeyError> {
        let vault = self.vault();
        blocking(move || {
            let item = vault.read_item(&id)?.ok_or(KeyError::NoSuchSecret)?;
            login::copy_password_of(&item).ok_or(KeyError::NoSuchSecret)
        })
        .await?
    }
    /// The counters of the logins API (module docs).
    #[must_use]
    pub fn login_counts(&self) -> LoginCounts {
        LoginCounts {
            lookups: self.login_counters.lookups.load(Ordering::SeqCst),
            password_reads: self.login_counters.password_reads.load(Ordering::SeqCst),
        }
    }

    /// Every login in the vault (Vibe Work's and the user's from other Vault apps), by name. Metadata only; the items
    /// are decrypted in memory and their passwords dropped at once.
    ///
    /// # Errors
    /// [`KeyError::Vault`] (locked, storage).
    pub async fn logins(&self) -> Result<Vec<Login>, KeyError> {
        self.login_counters.lookups.fetch_add(1, Ordering::SeqCst);
        let vault = self.vault();
        let items = blocking(move || vault.list_items()).await??;
        let mut out: Vec<Login> = items.iter().filter_map(login::from_item).collect();
        out.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(out)
    }

    /// Stores a login Vibe Work owns. An own login with the same origin and user name is replaced (same id).
    ///
    /// # Errors
    /// [`KeyError::InvalidSecret`] (address, user name, name or password out of bounds), [`KeyError::Vault`].
    pub async fn store_login(
        &self,
        name: String,
        url: String,
        username: String,
        password: SecretString,
    ) -> Result<StoredLogin, KeyError> {
        self.store_login_mode(name, url, username, password, true)
            .await
    }

    /// Creates a new login, refusing a site and username already seen. Its new id prevents overwriting an existing
    /// item; concurrent creators can still produce two logins for the same account.
    ///
    /// # Errors
    /// Invalid inputs, an existing login, or a vault failure.
    pub async fn create_login(
        &self,
        name: String,
        url: String,
        username: String,
        password: SecretString,
    ) -> Result<StoredLogin, KeyError> {
        self.store_login_mode(name, url, username, password, false)
            .await
    }

    async fn store_login_mode(
        &self,
        name: String,
        url: String,
        username: String,
        password: SecretString,
        replace: bool,
    ) -> Result<StoredLogin, KeyError> {
        let Some((origin, _)) = login::site_of(&url) else {
            return Err(KeyError::InvalidSecret(
                "the site must be a web address such as https://example.com",
            ));
        };
        if !(login::MIN_PASSWORD..=login::MAX_PASSWORD).contains(&password.len()) {
            return Err(KeyError::InvalidSecret(
                "the password must be 8 to 4096 bytes (Vibe Work can hide only passwords of 8 or more)",
            ));
        }
        if username.chars().count() > login::MAX_USERNAME || name.chars().count() > 80 {
            return Err(KeyError::InvalidSecret(
                "the name (80) or the user name (200 characters) is too long",
            ));
        }
        password.register_as(vw_secrets::SecretKind::VaultValue);
        let vault = self.vault();
        blocking(move || {
            let items = vault.list_items()?;
            let wanted_user = username.trim().to_owned();
            let same = items.iter().filter_map(login::from_item).find(|l| {
                l.own && l.origin.eq_ignore_ascii_case(&origin) && l.username == wanted_user
            });
            let (id, created, replaced) = match same {
                Some(found) if replace => {
                    let created = items
                        .iter()
                        .find(|i| i.id == found.id)
                        .map(|i| i.created_ms);
                    (found.id, created, true)
                }
                Some(_) => return Err(KeyError::NameTaken),
                None => (login::new_id(), None, false),
            };
            let new = NewLogin {
                name: &name,
                url: &url,
                username: &username,
                password: &password,
            };
            let item = login::item(&id, &new, created, now_ms()).ok_or(KeyError::InvalidSecret(
                "the login is not acceptable (site, user name or password)",
            ))?;
            vault.upsert_item(&item)?;
            let stored = login::from_item(&item).ok_or(KeyError::Internal)?;
            Ok(StoredLogin {
                login: stored,
                replaced,
            })
        })
        .await?
    }

    /// Removes a login Vibe Work created. Logins of the user's other Vault apps are never removed here.
    ///
    /// # Errors
    /// [`KeyError::NotOwnSecret`], [`KeyError::Vault`].
    pub async fn remove_login(&self, id: String) -> Result<(), KeyError> {
        if !login::is_own(&id) {
            return Err(KeyError::NotOwnSecret);
        }
        let vault = self.vault();
        blocking(move || vault.delete_item(&id).map(|_| ()))
            .await?
            .map_err(KeyError::from)
    }

    /// The password of login `id` for the core's browser fill, registered as a stored secret, counted. Called only
    /// after every check of the fill passed and the user's grant holds.
    ///
    /// # Errors
    /// [`KeyError::NoSuchSecret`] (absent, not a login, no password, shorter than 8 bytes), [`KeyError::Vault`].
    pub async fn login_password(&self, id: String) -> Result<SecretString, KeyError> {
        self.login_counters
            .password_reads
            .fetch_add(1, Ordering::SeqCst);
        let vault = self.vault();
        let item = blocking(move || vault.read_item(&id))
            .await??
            .ok_or(KeyError::NoSuchSecret)?;
        login::password_of(&item).ok_or(KeyError::NoSuchSecret)
    }

    /// Every fill grant in the vault (revoked ones included: callers filter with [`FillGrant::active`]), newest grant
    /// first.
    ///
    /// # Errors
    /// [`KeyError::Vault`].
    pub async fn fill_grants(&self) -> Result<Vec<FillGrant>, KeyError> {
        let vault = self.vault();
        let items = blocking(move || vault.list_items()).await??;
        let mut out: Vec<FillGrant> = items.iter().filter_map(fill_grant::from_item).collect();
        out.sort_by_key(|g| std::cmp::Reverse(g.granted_ms));
        Ok(out)
    }

    /// The fill grant `id`, read from the vault now (`None`: absent or not a valid grant).
    ///
    /// # Errors
    /// [`KeyError::Vault`] (locked, storage, a vault version refused by the head guard).
    pub async fn fill_grant(&self, id: String) -> Result<Option<FillGrant>, KeyError> {
        if !fill_grant::is_grant_id(&id) {
            return Ok(None);
        }
        let vault = self.vault();
        let item = blocking(move || vault.read_item(&id)).await??;
        Ok(item.as_ref().and_then(fill_grant::from_item))
    }

    /// Creates or changes fill grant `id` atomically (as [`KeyService::update_command_grant`]): `change` gets the
    /// record as the vault holds it at the moment of the write and returns the record to write, or `None`.
    ///
    /// # Errors
    /// [`KeyError::InvalidSecret`] for an id that is not a fill grant id, [`KeyError::Vault`].
    pub async fn update_fill_grant(
        &self,
        id: String,
        change: impl Fn(Option<FillGrant>) -> Option<FillGrant> + Send + 'static,
    ) -> Result<Option<FillGrant>, KeyError> {
        if !fill_grant::is_grant_id(&id) {
            return Err(KeyError::InvalidSecret("not a fill grant id"));
        }
        let vault = self.vault();
        let key = id.clone();
        let item = blocking(move || {
            vault.update_item(&key, |current| {
                // A record at this id that is not a valid grant counts as absent (it never permits anything).
                let next = change(current.as_ref().and_then(fill_grant::from_item))?;
                if next.id != key {
                    return None;
                }
                fill_grant::resource(&next, now_ms()).to_item().ok()
            })
        })
        .await??;
        Ok(item.as_ref().and_then(fill_grant::from_item))
    }
}
