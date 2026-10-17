//! Hidden per-project Vault entry permissions. This changes only Vibe Work's record, never a foreign entry.
//!
//! Depends on: `vw-vault::entry_permission`, `service`. Called by: core list, user command and agent paths.

use vw_vault::entry_permission::{self, EntryPermission, EntryPolicy};
use vw_vault::{login, project_secret};

use crate::error::KeyError;
use crate::service::{KeyService, blocking, now_ms};

impl KeyService {
    /// All valid project records for one entry. Used only where an MCP forward request has no project identity:
    /// any Never blocks forwarding, without treating a project's Allowed as a global grant.
    ///
    /// # Errors
    /// Vault read failures and malformed reserved policy records are refused.
    pub async fn entry_permissions_for(
        &self,
        entry_id: &str,
    ) -> Result<Vec<EntryPolicy>, KeyError> {
        let vault = self.vault();
        let items = blocking(move || vault.list_items_strict())
            .await?
            .map_err(KeyError::permissions_unreadable)?;
        let mut policies = Vec::new();
        for item in &items {
            if item.id.starts_with(entry_permission::ID_PREFIX) {
                let policy =
                    entry_permission::from_item(item).ok_or(KeyError::PermissionsUnreadable)?;
                if policy.entry_id == entry_id {
                    policies.push(policy);
                }
            }
        }
        Ok(policies)
    }

    /// All explicit project records, used to render exceptions without reading values.
    ///
    /// # Errors
    /// Malformed reserved records and vault read failures are refused.
    pub async fn all_entry_permissions(&self) -> Result<Vec<EntryPolicy>, KeyError> {
        let vault = self.vault();
        let items = blocking(move || vault.list_items()).await??;
        items
            .iter()
            .filter(|item| item.id.starts_with(entry_permission::ID_PREFIX))
            .map(|item| entry_permission::from_item(item).ok_or(KeyError::PermissionsUnreadable))
            .collect()
    }

    /// Reads the current project policy. An absent valid record defaults to Ask each time; a malformed occupied
    /// policy slot fails closed rather than silently permitting a previously refused entry.
    ///
    /// # Errors
    /// Vault read failures or a malformed occupied policy slot are refused.
    pub async fn entry_permission(
        &self,
        project: &str,
        entry_id: &str,
    ) -> Result<(EntryPermission, i64), KeyError> {
        Ok(self
            .own_entry_permission(project, entry_id)
            .await?
            .unwrap_or(self.default_entry_permission(entry_id).await?))
    }

    /// Reads an explicit project record, if present, without applying the default.
    ///
    /// # Errors
    /// Vault read failures or a malformed occupied policy slot are refused.
    pub async fn own_entry_permission(
        &self,
        project: &str,
        entry_id: &str,
    ) -> Result<Option<(EntryPermission, i64)>, KeyError> {
        let id = entry_permission::id_for(project, entry_id);
        let vault = self.vault();
        let item = blocking(move || vault.read_item(&id))
            .await?
            .map_err(KeyError::permissions_unreadable)?;
        match item {
            None => Ok(None),
            Some(item) => entry_permission::from_item(&item)
                .filter(|policy| {
                    policy.entry_id == entry_id
                        && entry_permission::id_for(project, entry_id) == policy.id
                })
                .map(|policy| Some((policy.permission, policy.changed_ms)))
                .ok_or(KeyError::PermissionsUnreadable),
        }
    }

    /// Writes a policy only for an existing listed secret or login. The entry itself can belong to another app.
    ///
    /// # Errors
    /// Invalid entries, malformed policy slots and Vault write failures are refused.
    pub async fn set_entry_permission(
        &self,
        project: String,
        entry_id: String,
        permission: EntryPermission,
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
            let key = id.clone();
            let item = vault.update_item(&id, move |current| {
                // A malformed occupied slot is never overwritten automatically.
                if current.is_some()
                    && current
                        .as_ref()
                        .and_then(entry_permission::from_item)
                        .is_none()
                {
                    return None;
                }
                let previous = current.as_ref().and_then(entry_permission::from_item);
                let changed_ms = now_ms().max(
                    previous
                        .as_ref()
                        .map_or(0, |p| p.changed_ms.saturating_add(1)),
                );
                let policy = EntryPolicy {
                    id: key.clone(),
                    project: project.clone(),
                    entry_id: entry_id.clone(),
                    permission,
                    changed_ms,
                    created_ms: previous.map_or(changed_ms, |p| p.created_ms),
                };
                entry_permission::resource(&policy).to_item().ok()
            })?;
            if item
                .as_ref()
                .and_then(entry_permission::from_item)
                .is_some_and(|p| p.permission == permission)
            {
                Ok(())
            } else {
                Err(KeyError::InvalidSecret("entry permission was not stored"))
            }
        })
        .await?
    }
}
