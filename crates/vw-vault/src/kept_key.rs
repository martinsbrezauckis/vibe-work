//! The vault's kept key (the device key in the OS credential store plus the keyfile rewrapped under it in
//! `profile.json`): finding it for the selected vault and deleting it. Lock and the lock-at-quit switch delete it;
//! a quit that keeps the last state leaves it.
//!
//! Depends on: the parent `vault` module (its private state), `layout`, `shellx_profile`. Called by: `Vault::lock`,
//! vw-keys (start with the switch on, stale records).

use crate::error::VaultError;
use crate::layout::{Detected, LocalVault, detect};
use crate::shellx_profile::ShellxProfile;

use super::Vault;

impl Vault {
    /// Deletes the kept device key of the selected (or only) vault: first the rewrapped keyfile
    /// in its profile, then the credential store entry. `Ok(true)` when the profile still held a kept key.
    ///
    /// # Errors
    /// Profile I/O errors, or [`VaultError::Remember`] when the credential store fails (the
    /// profile is already cleared then, so the entry no longer opens the vault).
    pub fn forget_kept_key(&self) -> Result<bool, VaultError> {
        let Some((mut profile, profile_dir)) = self.kept_key_profile()? else {
            return Ok(false);
        };
        let in_profile = profile.remember_device || profile.remembered_keyfile_json.is_some();
        if in_profile {
            profile.remember_device = false;
            profile.remembered_keyfile_json = None;
            profile.write(&profile_dir)?;
        }
        // Deleting an absent entry is fine; no read first (one credential store call, no secret read).
        self.config
            .device_store
            .delete(&self.device_account(&profile_dir))?;
        Ok(in_profile)
    }

    /// The selected (or only) ShellX-layout profile and its folder; none for the Vault app's
    /// layout or before a choice.
    fn kept_key_profile(&self) -> Result<Option<(ShellxProfile, std::path::PathBuf)>, VaultError> {
        if let Some(managed) = &self.managed {
            let Some(kind) = managed.selected(&self.config.location)? else {
                return Ok(None);
            };
            return Ok(managed
                .available(&self.config.location)?
                .into_iter()
                .find(|entry| entry.kind == kind)
                .and_then(|entry| match entry.vault {
                    LocalVault::Shellx { profile, .. } => Some((*profile, entry.path)),
                    LocalVault::Desktop { .. } => None,
                }));
        }
        Ok(match detect(&self.config.location)? {
            Detected::One(LocalVault::Shellx { profile, .. }) => {
                Some((*profile, self.config.location.profile_dir.clone()))
            }
            _ => None,
        })
    }
}
