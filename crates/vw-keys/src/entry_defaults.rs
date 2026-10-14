//! Hidden entry-wide defaults, sealed by the same Vault snapshot as project policy records.
//!
//! Depends on: `vw-vault::entry_permission`, `service`. Called by core listing, session capture and agent checks.

use std::collections::BTreeMap;

use vw_vault::entry_permission::{self, DefaultPolicy, EntryPermission};
use vw_vault::{login, project_secret};

use crate::error::KeyError;
use crate::service::{KeyService, blocking, now_ms};

impl KeyService {
    /// All valid defaults at this instant; malformed reserved slots fail closed.
    ///
    /// # Errors
    /// Vault read failures or malformed default records are refused.
    pub async fn entry_defaults(&self) -> Result<BTreeMap<String, EntryPermission>, KeyError> {
        let vault = self.vault();
        let items = blocking(move || vault.list_items()).await??;
        let mut defaults = BTreeMap::new();
        for item in &items {
            if item.id.starts_with(entry_permission::DEFAULT_ID_PREFIX) {
                let policy = entry_permission::default_from_item(item)
                    .ok_or(KeyError::InvalidSecret("invalid entry default record"))?;
                defaults.insert(policy.entry_id, policy.permission);
            }
        }
        Ok(defaults)
    }

    /// The current entry-wide choice; missing records migrate to Ask each time.
    ///
    /// # Errors
    /// Vault read failures or malformed occupied slots are refused.
    pub async fn default_entry_permission(
        &self,
        entry_id: &str,
    ) -> Result<(EntryPermission, i64), KeyError> {
        let id = entry_permission::default_id_for(entry_id);
        let vault = self.vault();
        let item = blocking(move || vault.read_item(&id)).await??;
        match item {
            None => Ok((EntryPermission::AskEachTime, 0)),
            Some(item) => entry_permission::default_from_item(&item)
                .filter(|p| p.entry_id == entry_id)
                .map(|p| (p.permission, p.changed_ms))
                .ok_or(KeyError::InvalidSecret("invalid entry default record")),
        }
    }

    /// Sets a default only for a listed agent-visible secret or login.
    ///
    /// # Errors
    /// Invalid entries, malformed slots and vault write failures are refused.
    pub async fn set_default_entry_permission(
        &self,
        entry_id: String,
        permission: EntryPermission,
    ) -> Result<(), KeyError> {
        if entry_id.is_empty() {
            return Err(KeyError::InvalidSecret("entry is required"));
        }
        let vault = self.vault();
        blocking(move || {
            let entry = vault.read_item(&entry_id)?.ok_or(KeyError::NoSuchSecret)?;
            if project_secret::agent_visible(&entry).is_none() && login::from_item(&entry).is_none()
            {
                return Err(KeyError::NoSuchSecret);
            }
            let id = entry_permission::default_id_for(&entry_id);
            let key = id.clone();
            let item = vault.update_item(&id, move |current| {
                if current.is_some()
                    && current
                        .as_ref()
                        .and_then(entry_permission::default_from_item)
                        .is_none()
                {
                    return None;
                }
                let previous = current
                    .as_ref()
                    .and_then(entry_permission::default_from_item);
                let changed_ms = now_ms().max(
                    previous
                        .as_ref()
                        .map_or(0, |p| p.changed_ms.saturating_add(1)),
                );
                let policy = DefaultPolicy {
                    id: key.clone(),
                    entry_id: entry_id.clone(),
                    permission,
                    changed_ms,
                    created_ms: previous.map_or(changed_ms, |p| p.created_ms),
                };
                entry_permission::default_resource(&policy).to_item().ok()
            })?;
            if item
                .as_ref()
                .and_then(entry_permission::default_from_item)
                .is_some_and(|p| p.permission == permission)
            {
                Ok(())
            } else {
                Err(KeyError::InvalidSecret("entry default was not stored"))
            }
        })
        .await?
    }

    /// Removes one explicit project choice, restoring inheritance from the default.
    ///
    /// # Errors
    /// Malformed slots, invalid entries and vault write failures are refused.
    pub async fn reset_entry_permission(
        &self,
        project: String,
        entry_id: String,
    ) -> Result<(), KeyError> {
        if project.trim().is_empty() || entry_id.is_empty() {
            return Err(KeyError::InvalidSecret("project and entry are required"));
        }
        let vault = self.vault();
        blocking(move || {
            let entry = vault.read_item(&entry_id)?.ok_or(KeyError::NoSuchSecret)?;
            if project_secret::agent_visible(&entry).is_none() && login::from_item(&entry).is_none()
            {
                return Err(KeyError::NoSuchSecret);
            }
            let id = entry_permission::id_for(&project, &entry_id);
            let deleted = vault.delete_item_when(&id, |item| {
                entry_permission::from_item(item)
                    .is_some_and(|p| p.id == id && p.entry_id == entry_id)
            })?;
            if deleted == Some(false) {
                return Err(KeyError::InvalidSecret("invalid entry permission record"));
            }
            Ok(())
        })
        .await?
    }
}
