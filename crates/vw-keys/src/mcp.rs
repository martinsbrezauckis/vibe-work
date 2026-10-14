//! MCP server key bindings on the key service: list, store and remove the vault records that tell the
//! MCP forwarder which secret goes into which header for which server, and install the core's resolver in the proxy.
//!
//! Depends on: vw-vault (`mcp_binding`, the vault), vw-keyproxy (`McpResolver`), `service`. Called by: vw-core-api
//! (`mcp_keys`).
//! Invariants: bindings carry metadata only (the key is read with [`KeyService::secret_value`] by the resolver, per
//! request); a locked vault answers [`KeyError::Vault`] `Locked`, nothing waits here.

use std::sync::{Arc, PoisonError};

use vw_vault::mcp_binding::{self, McpBinding};

use crate::error::KeyError;
use crate::service::{KeyService, blocking, now_ms};

impl KeyService {
    /// Every MCP server key binding in the vault, by server name.
    ///
    /// # Errors
    /// [`KeyError::Vault`] (locked, storage).
    pub async fn mcp_bindings(&self) -> Result<Vec<McpBinding>, KeyError> {
        let vault = self.vault();
        let items = blocking(move || vault.list_items()).await??;
        let mut out: Vec<McpBinding> = items.iter().filter_map(mcp_binding::from_item).collect();
        out.sort_by(|a, b| a.server.cmp(&b.server));
        Ok(out)
    }

    /// The binding `id`, read from the vault now (the forwarder decides every request on the vault's
    /// current record, never on a copy). `None` when there is no such binding.
    ///
    /// # Errors
    /// [`KeyError::Vault`] (locked, storage).
    pub async fn mcp_binding(&self, id: String) -> Result<Option<McpBinding>, KeyError> {
        if !mcp_binding::is_binding_id(&id) {
            return Ok(None);
        }
        let vault = self.vault();
        let item = blocking(move || vault.read_item(&id)).await??;
        Ok(item.as_ref().and_then(mcp_binding::from_item))
    }

    /// Stores (adds or replaces) `binding`.
    ///
    /// # Errors
    /// [`KeyError::InvalidSecret`] for an id that is not a binding id, [`KeyError::Vault`].
    pub async fn store_mcp_binding(&self, binding: McpBinding) -> Result<(), KeyError> {
        if !mcp_binding::is_binding_id(&binding.id) {
            return Err(KeyError::InvalidSecret("not an MCP binding id"));
        }
        let vault = self.vault();
        blocking(move || {
            let item = mcp_binding::resource(&binding, now_ms()).to_item()?;
            vault.upsert_item(&item).map(|_| ())
        })
        .await?
        .map_err(KeyError::from)
    }

    /// Changes binding `id` atomically (to prevent a time-of-check/time-of-use race): `change` gets the record as it is in the
    /// vault at the moment of the write (under the vault's lock, re-read when another app moved the head) and returns
    /// true to write it. Returns the record as it is afterwards (`None`: no such binding). A grant computed from an
    /// older read can therefore never overwrite a revoke made in between, in this process or another.
    ///
    /// # Errors
    /// [`KeyError::Vault`] (locked, storage, conflict).
    pub async fn update_mcp_binding(
        &self,
        id: String,
        change: impl Fn(&mut McpBinding) -> bool + Send + 'static,
    ) -> Result<Option<McpBinding>, KeyError> {
        if !mcp_binding::is_binding_id(&id) {
            return Ok(None);
        }
        let vault = self.vault();
        let item = blocking(move || {
            vault.update_item(&id, |current| {
                let mut binding = mcp_binding::from_item(current.as_ref()?)?;
                if !change(&mut binding) {
                    return None;
                }
                mcp_binding::resource(&binding, now_ms()).to_item().ok()
            })
        })
        .await??;
        Ok(item.as_ref().and_then(mcp_binding::from_item))
    }

    /// Removes binding `id` (only binding ids are accepted).
    ///
    /// # Errors
    /// [`KeyError::InvalidSecret`], [`KeyError::Vault`].
    pub async fn remove_mcp_binding(&self, id: String) -> Result<(), KeyError> {
        if !mcp_binding::is_binding_id(&id) {
            return Err(KeyError::InvalidSecret("not an MCP binding id"));
        }
        let vault = self.vault();
        blocking(move || vault.delete_item(&id).map(|_| ()))
            .await?
            .map_err(KeyError::from)
    }

    /// Installs the core's MCP resolver in the proxy; until then every MCP request answers 503.
    pub fn set_mcp_resolver(&self, resolver: Arc<dyn vw_keyproxy::McpResolver>) {
        *self.mcp.write().unwrap_or_else(PoisonError::into_inner) = Some(resolver);
    }

    /// Removes the resolver (core shutdown): MCP requests answer 503 again.
    pub fn clear_mcp_resolver(&self) {
        *self.mcp.write().unwrap_or_else(PoisonError::into_inner) = None;
    }
}
