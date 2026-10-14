//! Project secrets on the key service: list the
//! secrets an agent may name, add or replace one, remove one Vibe Work created, and read a value for the broker.
//!
//! Depends on: vw-vault (`project_secret`, the vault), `service`. Called by: vw-core-api (`secrets`, the agent tool
//! `secret_run`, the browser capture).
//! Invariants:
//! - Lists and results carry metadata only ([`ProjectSecret`]). The only function that returns a value is
//!   [`KeyService::secret_value`], for the core's broker, which puts it into one child process's environment.
//! - Every value this module touches is registered as a stored secret (`vaultValue`) before it is returned or
//!   stored, so no Debug API answer and no agent tool answer carries it; listing also
//!   registers every agent-visible value, so a stored key the agent meets elsewhere (a web page) is hidden too.
//! - A locked vault answers [`KeyError::Vault`] `Locked`; nothing waits here.

use vw_vault::items::VaultResource;
use vw_vault::project_secret::{self, ProjectSecret, SecretLabel};
use vw_vault::{SecretString, VaultError};

use crate::error::KeyError;
use crate::service::{KeyService, blocking, now_ms};

/// How a store call went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// The secret (metadata).
    pub secret: ProjectSecret,
    /// True when an own secret of the same name was replaced.
    pub replaced: bool,
}

impl KeyService {
    /// Renames an own secret, preserving its id and every other resource field.
    ///
    /// # Errors
    /// Invalid names, another secret's name, missing or foreign items, and vault failures.
    pub async fn rename_secret(&self, id: String, name: String) -> Result<(), KeyError> {
        if !project_secret::valid_name(&name) {
            return Err(KeyError::InvalidSecret(
                "the name must be 1 to 80 characters of text",
            ));
        }
        let vault = self.vault();
        blocking(move || {
            let items = vault.list_items()?;
            let current = items
                .iter()
                .find(|item| item.id == id)
                .ok_or(KeyError::NoSuchSecret)?;
            let own = project_secret::agent_visible(current).is_some_and(|s| s.own);
            if !own {
                return Err(KeyError::NotOwnSecret);
            }
            let wanted = name.trim().to_lowercase();
            if items.iter().any(|item| {
                item.id != id
                    && project_secret::agent_visible(item)
                        .is_some_and(|s| s.name.to_lowercase() == wanted)
            }) {
                return Err(KeyError::NameTaken);
            }
            let changed = vault.update_item(&id, |current| {
                let mut resource = VaultResource::from_item(&current?)?;
                if !project_secret::is_own(&resource.id) {
                    return None;
                }
                name.trim().clone_into(&mut resource.label);
                resource.updated_ms = now_ms();
                resource.to_item().ok()
            })?;
            changed.ok_or(KeyError::NoSuchSecret)?;
            Ok(())
        })
        .await?
    }

    /// Replaces only the value field of an own secret, preserving metadata and id.
    ///
    /// # Errors
    /// Invalid values, missing or foreign items, and vault failures.
    pub async fn replace_secret(&self, id: String, value: SecretString) -> Result<(), KeyError> {
        if value.is_empty() || value.len() > project_secret::MAX_VALUE {
            return Err(KeyError::InvalidSecret("the value must be 1 to 4096 bytes"));
        }
        let vault = self.vault();
        blocking(move || {
            let current = vault.read_item(&id)?.ok_or(KeyError::NoSuchSecret)?;
            if !project_secret::agent_visible(&current).is_some_and(|s| s.own) {
                return Err(KeyError::NotOwnSecret);
            }
            let changed = vault.update_item(&id, |current| {
                let mut resource = VaultResource::from_item(&current?)?;
                if !project_secret::is_own(&resource.id) {
                    return None;
                }
                resource.secret_fields.insert(
                    project_secret::VALUE_FIELD.to_owned(),
                    value.expose().to_owned(),
                );
                resource.updated_ms = now_ms();
                resource.to_item().ok()
            })?;
            changed.ok_or(KeyError::NoSuchSecret)?;
            value.register_as(vw_secrets::SecretKind::VaultValue);
            Ok(())
        })
        .await?
    }

    /// Reads a listed secret's value for a user-initiated clipboard operation.
    ///
    /// # Errors
    /// Missing or non-agent-visible items and vault failures.
    pub async fn listed_secret_value(&self, id: String) -> Result<SecretString, KeyError> {
        let vault = self.vault();
        blocking(move || {
            let item = vault.read_item(&id)?.ok_or(KeyError::NoSuchSecret)?;
            project_secret::copy_value_of(&item).ok_or(KeyError::NoSuchSecret)
        })
        .await?
    }
    /// The secrets an agent may name, by name (metadata only).
    ///
    /// # Errors
    /// [`KeyError::Vault`] (locked, storage).
    pub async fn project_secrets(&self) -> Result<Vec<ProjectSecret>, KeyError> {
        let vault = self.vault();
        let items = blocking(move || vault.list_items()).await??;
        let mut out: Vec<ProjectSecret> = items
            .iter()
            .filter_map(|item| {
                let meta = project_secret::agent_visible(item)?;
                // Registered now (module invariant); the value is dropped right away.
                for field in &meta.fields {
                    let _ = project_secret::value_of(item, Some(field));
                }
                Some(meta)
            })
            .collect();
        out.sort_by_key(|a| a.name.to_lowercase());
        Ok(out)
    }

    /// Stores `value` under `name` as a Vibe Work secret. An own secret with the same name (case-insensitive) is
    /// replaced (same id); a secret of another app with that name is left alone and refused
    /// ([`KeyError::NameTaken`]). `source_origin` records where a captured key came from.
    ///
    /// # Errors
    /// [`KeyError::InvalidSecret`] (name or value), [`KeyError::NameTaken`], [`KeyError::Vault`].
    pub async fn store_secret(
        &self,
        name: String,
        value: SecretString,
        source_origin: Option<String>,
    ) -> Result<Stored, KeyError> {
        self.store_secret_labelled(name, value, source_origin, None)
            .await
    }

    /// [`Self::store_secret`] with a [`SecretLabel`]. The label is a suggestion only: it is stored with
    /// the metadata and changes nothing else. When an own secret is replaced and `label` is `None`, its label is kept.
    ///
    /// # Errors
    /// As [`Self::store_secret`].
    pub async fn store_secret_labelled(
        &self,
        name: String,
        value: SecretString,
        source_origin: Option<String>,
        label: Option<SecretLabel>,
    ) -> Result<Stored, KeyError> {
        self.store_secret_labelled_mode(name, value, source_origin, label, true)
            .await
    }

    /// Creates a new named secret, refusing a name already seen. The new id prevents overwriting an existing item;
    /// concurrent creators can still produce two items with the same name.
    ///
    /// # Errors
    /// Invalid names or values, a taken name, or a vault failure.
    pub async fn create_secret_labelled(
        &self,
        name: String,
        value: SecretString,
        label: Option<SecretLabel>,
    ) -> Result<Stored, KeyError> {
        self.store_secret_labelled_mode(name, value, None, label, false)
            .await
    }

    async fn store_secret_labelled_mode(
        &self,
        name: String,
        value: SecretString,
        source_origin: Option<String>,
        label: Option<SecretLabel>,
        replace: bool,
    ) -> Result<Stored, KeyError> {
        if !project_secret::valid_name(&name) {
            return Err(KeyError::InvalidSecret(
                "the name must be 1 to 80 characters of text",
            ));
        }
        if value.is_empty() || value.len() > project_secret::MAX_VALUE {
            return Err(KeyError::InvalidSecret("the value must be 1 to 4096 bytes"));
        }
        value.register_as(vw_secrets::SecretKind::VaultValue);
        let vault = self.vault();
        blocking(move || {
            let items = vault.list_items()?;
            let wanted = name.trim().to_lowercase();
            let same = items.iter().find_map(|item| {
                project_secret::agent_visible(item).filter(|m| m.name.to_lowercase() == wanted)
            });
            let (id, created, replaced, kept_label) = match same {
                Some(meta) if meta.own && replace => {
                    let created = items.iter().find(|i| i.id == meta.id).map(|i| i.created_ms);
                    (meta.id, created, true, meta.label)
                }
                Some(_) => return Err(KeyError::NameTaken),
                None => (project_secret::new_id(), None, false, None),
            };
            let resource = project_secret::resource(
                &id,
                &name,
                &value,
                source_origin.as_deref(),
                label.or(kept_label),
                created,
                now_ms(),
            );
            vault.upsert_item(&resource.to_item()?)?;
            let item = resource.to_item()?;
            let secret = project_secret::agent_visible(&item).ok_or(KeyError::Internal)?;
            Ok(Stored { secret, replaced })
        })
        .await?
    }

    /// Removes a secret Vibe Work created. Secrets of the user or another app are never removed here.
    ///
    /// # Errors
    /// [`KeyError::NotOwnSecret`], [`KeyError::Vault`].
    pub async fn remove_secret(&self, id: String) -> Result<(), KeyError> {
        if !project_secret::is_own(&id) {
            return Err(KeyError::NotOwnSecret);
        }
        let vault = self.vault();
        blocking(move || vault.delete_item(&id).map(|_| ()))
            .await?
            .map_err(KeyError::from)
    }

    /// The value of an agent-visible secret for the broker (registered as a stored secret). `field` picks a secret
    /// field (default: `value`, else the only one).
    ///
    /// # Errors
    /// [`KeyError::NoSuchSecret`] (absent, `userOnly`, not a developer credential or secret, empty field),
    /// [`KeyError::Vault`].
    pub async fn secret_value(
        &self,
        id: String,
        field: Option<String>,
    ) -> Result<SecretString, KeyError> {
        let vault = self.vault();
        let item = blocking(move || vault.read_item(&id))
            .await??
            .ok_or(KeyError::NoSuchSecret)?;
        project_secret::value_of(&item, field.as_deref()).ok_or(KeyError::NoSuchSecret)
    }
}

/// True when `error` is the vault being locked (callers ask the user to unlock).
#[must_use]
pub fn is_locked(error: &KeyError) -> bool {
    matches!(error, KeyError::Vault(VaultError::Locked))
}
