//! The vault's last state at start, and the "Lock the vault when Vibe Work quits" switch.
//!
//! At start, before any unlock: the switch is read (an install from before it takes it from its old "remember this
//! computer" choice, once, see `vault_settings`). Switch on: a kept key found now is one a run that did not quit left
//! behind (crash, power off, killed process), so it is deleted and the vault starts locked. Switch off: the kept key
//! opens the vault; a kept-key record whose key the credential store no longer has is cleared. An install updated
//! from before the switch with a kept key starts locked, and the kept key opens nothing until the password is typed
//! once (its old Lock left no trace, see `vault_settings`). The quit side is `KeyService::shutdown`.
//!
//! Depends on: vw-vault, `vault_settings`. Called by: `service` (start, status, the switch command).

use tracing::{info, warn};
use vw_vault::{Vault, VaultStatus};

use super::KeyService;
use crate::error::KeyError;
use crate::vault_settings::{AtStart, VaultSettings};

/// Restores the vault's last state (blocking); returns what the start read.
pub(super) fn restore(vault: &Vault, settings: &VaultSettings) -> AtStart {
    // The old choice counts only for the vault this install uses (a vault found but not chosen yet was never its
    // choice).
    let selected = matches!(vault.selected_vault(), Ok(Some(_)));
    let remembered = match vault.status() {
        Ok(VaultStatus::Locked {
            remembered_device, ..
        }) if selected => Some(remembered_device),
        _ => None,
    };
    let at_start = settings.at_start(remembered);
    if at_start.lock_on_quit {
        match vault.forget_kept_key() {
            Ok(true) => info!("key service: deleted the key a run that did not quit kept"),
            Ok(false) => {}
            Err(error) => warn!(%error, "key service: the kept key was not deleted"),
        }
        return at_start;
    }
    if at_start.locked_until_password {
        info!("key service: the vault starts locked until its password is typed once");
        return at_start;
    }
    match vault.unlock_remembered() {
        Ok(true) => info!("key service: vault unlocked with the kept key"),
        // The profile says a key is kept but the credential store has none (no Secret Service, or the entry was
        // deleted): the record is stale, so the vault is not reported as having a kept key.
        Ok(false) if remembered == Some(true) => {
            if let Err(error) = vault.forget_kept_key() {
                warn!(%error, "key service: a stale kept-key record was not cleared");
            }
        }
        Ok(false) => {}
        Err(error) => warn!(%error, "key service: unlock with the kept key failed"),
    }
    at_start
}

impl KeyService {
    /// "Lock the vault when Vibe Work quits".
    #[must_use]
    pub fn lock_on_quit(&self) -> bool {
        self.lock_on_quit.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// True while the kept key may not open the vault (an updated install before its first password unlock).
    pub(super) fn waits_for_password(&self) -> bool {
        self.locked_until_password
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// After a password unlock or a new vault: the kept key may open the vault again.
    pub(super) fn password_typed(&self) {
        if self
            .locked_until_password
            .swap(false, std::sync::atomic::Ordering::SeqCst)
            && let Err(error) = self.vault_settings.end_wait_for_password()
        {
            warn!(%error, "key service: the end of the wait for the password was not saved");
        }
    }

    /// Turns "Lock the vault when Vibe Work quits" on or off. Nothing changes now: an unlocked
    /// vault stays unlocked (and keeps its key) until the quit.
    ///
    /// # Errors
    /// [`KeyError::Settings`] when the file cannot be written (the switch is unchanged then).
    pub fn set_lock_on_quit(&self, on: bool) -> Result<(), KeyError> {
        self.vault_settings.set_lock_on_quit(on)?;
        self.lock_on_quit
            .store(on, std::sync::atomic::Ordering::SeqCst);
        // The network check key's copy exists only while the switch is off (`network`).
        crate::network::switch_changed(&self.vault, on);
        self.changed();
        Ok(())
    }
}
