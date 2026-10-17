//! The secret broker's command grants on the key service: list, read and change the vault records of
//! the "always for this command" answers (`vw_vault::command_grant`).
//!
//! Depends on: vw-vault (`command_grant`, the vault), `service`. Called by: vw-core-api (`secret_grants`).
//! Invariants: grants carry metadata only; every read goes to the vault now (nothing is cached here, so a revoke by
//! another window or app on the same vault is seen at once); a locked vault answers [`KeyError::Vault`] `Locked`.

use vw_vault::command_grant::{self, CommandGrant};

use crate::error::KeyError;
use crate::service::{KeyService, blocking, now_ms};

impl KeyService {
    /// Every command grant in the vault (revoked ones included: callers filter with [`CommandGrant::active`]), newest
    /// grant first.
    ///
    /// # Errors
    /// [`KeyError::Vault`] (locked, storage).
    pub async fn command_grants(&self) -> Result<Vec<CommandGrant>, KeyError> {
        let vault = self.vault();
        let items = blocking(move || vault.list_items_strict())
            .await?
            .map_err(KeyError::permissions_unreadable)?;
        let mut out: Vec<CommandGrant> =
            items.iter().filter_map(command_grant::from_item).collect();
        out.sort_by_key(|g| std::cmp::Reverse(g.granted_ms));
        Ok(out)
    }

    /// The grant `id`, read from the vault now. `None` when there is no such grant (or the record is not a valid
    /// grant).
    ///
    /// # Errors
    /// [`KeyError::Vault`] (locked, storage, a vault version refused by the head guard).
    pub async fn command_grant(&self, id: String) -> Result<Option<CommandGrant>, KeyError> {
        if !command_grant::is_grant_id(&id) {
            return Ok(None);
        }
        let vault = self.vault();
        let item = blocking(move || vault.read_item(&id)).await??;
        Ok(item.as_ref().and_then(command_grant::from_item))
    }

    /// Creates or changes grant `id` atomically (as [`KeyService::update_mcp_binding`]): `change` gets the record as
    /// it is in the vault at the moment of the write (`None`: absent) and returns the record to write, or `None` to
    /// write nothing. A record at that id that is not a valid grant is passed as `None` (a new grant replaces it), and
    /// a returned record with another id is not written. Returns the record as it is afterwards (`None`: absent or
    /// not a valid grant).
    ///
    /// # Errors
    /// [`KeyError::InvalidSecret`] for an id that is not a grant id, [`KeyError::Vault`].
    pub async fn update_command_grant(
        &self,
        id: String,
        change: impl Fn(Option<CommandGrant>) -> Option<CommandGrant> + Send + 'static,
    ) -> Result<Option<CommandGrant>, KeyError> {
        if !command_grant::is_grant_id(&id) {
            return Err(KeyError::InvalidSecret("not a command grant id"));
        }
        let vault = self.vault();
        let key = id.clone();
        let item = blocking(move || {
            vault.update_item(&key, |current| {
                // A record at this id that is not a valid grant (damaged, or written by another app) counts as absent:
                // it never permits anything, and a new answer of the user replaces it.
                let next = change(current.as_ref().and_then(command_grant::from_item))?;
                if next.id != key {
                    return None;
                }
                command_grant::resource(&next, now_ms()).to_item().ok()
            })
        })
        .await??;
        Ok(item.as_ref().and_then(command_grant::from_item))
    }
}
