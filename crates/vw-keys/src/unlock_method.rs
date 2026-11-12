//! Vault unlock choice and password transitions. Called by the typed UI commands; persists policy before removing
//! an unlock secret, and never writes one in password mode. The password and all vault contents stay unchanged.
use super::{KeyService, blocking};
use crate::{KeyError, UnlockMethod};
use std::sync::{Arc, atomic::Ordering};
use vw_vault::{SecretString, VaultError, VaultKind};

impl KeyService {
    /// The current unlock choice (old installs default to system sign-in).
    #[must_use]
    pub fn unlock_method(&self) -> UnlockMethod {
        if self.unlock_with_password.load(Ordering::SeqCst) {
            UnlockMethod::Password
        } else {
            UnlockMethod::System
        }
    }

    fn save_unlock_method(&self, method: UnlockMethod) -> Result<(), KeyError> {
        self.vault_settings.set_unlock_method(method)?;
        self.unlock_with_password
            .store(method == UnlockMethod::Password, Ordering::SeqCst);
        self.changed();
        Ok(())
    }

    /// Creates a vault using the persisted choice (system sign-in on existing installs).
    /// # Errors
    /// Vault creation or settings I/O fails.
    pub async fn create_vault(&self, passphrase: SecretString) -> Result<(), KeyError> {
        self.create_vault_with_unlock(passphrase, None).await
    }

    /// Creates a vault with the person's explicit setup choice; absent preserves older clients' behavior.
    /// # Errors
    /// Vault creation or settings I/O fails. An existing vault is never changed by this request.
    pub async fn create_vault_with_unlock(
        &self,
        passphrase: SecretString,
        method: Option<UnlockMethod>,
    ) -> Result<(), KeyError> {
        let _change = self.unlock_change.lock().await;
        if self.vault.selected_vault()?.is_some() {
            return Err(VaultError::AlreadyExists.into());
        }
        if let Some(method) = method {
            self.save_unlock_method(method)?;
        }
        let remember = self.unlock_method() == UnlockMethod::System;
        let vault = Arc::clone(&self.vault);
        blocking(move || vault.create(&passphrase, remember)).await??;
        if remember && self.vault.remember_note().is_some() {
            self.save_unlock_method(UnlockMethod::Password)?;
        }
        self.password_typed();
        self.network_unlocked().await;
        self.refresh_key().await;
        Ok(())
    }

    /// Unlocks with a password, retaining the persisted choice on every subsequent unlock.
    /// # Errors
    /// Wrong password, selection errors, or vault I/O fails.
    pub async fn unlock(
        &self,
        passphrase: SecretString,
        choice: Option<VaultKind>,
    ) -> Result<(), KeyError> {
        self.unlock_with_method(passphrase, choice, None).await
    }

    /// First-run selection may carry an explicit unlock choice. A wrong password never changes that choice.
    /// # Errors
    /// Wrong password, settings I/O, selection errors or an unavailable vault.
    pub async fn unlock_with_method(
        &self,
        passphrase: SecretString,
        choice: Option<VaultKind>,
        method: Option<UnlockMethod>,
    ) -> Result<(), KeyError> {
        let _change = self.unlock_change.lock().await;
        if let Some(method) = method {
            let vault = Arc::clone(&self.vault);
            let pass = passphrase.clone();
            blocking(move || vault.verify_password_choice(&pass, choice)).await??;
            self.save_unlock_method(method)?;
        }
        let remember = self.unlock_method() == UnlockMethod::System;
        let vault = Arc::clone(&self.vault);
        blocking(move || vault.unlock(&passphrase, Some(remember), choice)).await??;
        if remember && self.vault.remember_note().is_some() {
            self.save_unlock_method(UnlockMethod::Password)?;
        }
        self.password_typed();
        self.network_unlocked().await;
        self.refresh_key().await;
        Ok(())
    }

    /// Tries the existing kept key only when the person's policy allows it.
    /// # Errors
    /// The credential store or saved key is unavailable.
    pub async fn unlock_remembered(&self) -> Result<bool, KeyError> {
        let _change = self.unlock_change.lock().await;
        if self.waits_for_password() || self.unlock_method() == UnlockMethod::Password {
            return Ok(false);
        }
        let vault = Arc::clone(&self.vault);
        let unlocked = blocking(move || vault.unlock_remembered_for_user()).await??;
        if unlocked {
            self.network_unlocked().await;
            self.refresh_key().await;
        }
        Ok(unlocked)
    }

    /// Changes the unlock method after checking the vault password, without rewriting any data items.
    /// # Errors
    /// Wrong password, settings or credential-store failure. A failed system transition rolls back its kept key.
    pub async fn set_unlock_method(
        &self,
        method: UnlockMethod,
        passphrase: SecretString,
    ) -> Result<(), KeyError> {
        let _change = self.unlock_change.lock().await;
        let vault = Arc::clone(&self.vault);
        let pass = passphrase.clone();
        blocking(move || vault.verify_password(&pass)).await??;
        if method == UnlockMethod::Password {
            // Durable refusal first: a crash or failed OS delete cannot make the next start unlock automatically.
            self.save_unlock_method(method)?;
            let vault = Arc::clone(&self.vault);
            blocking(move || vault.forget_kept_key()).await??;
        } else {
            let vault = Arc::clone(&self.vault);
            blocking(move || vault.keep_unlock_key(&passphrase)).await??;
            if let Err(error) = self.save_unlock_method(method) {
                let vault = Arc::clone(&self.vault);
                blocking(move || vault.forget_kept_key()).await??;
                return Err(error);
            }
            self.password_typed();
        }
        self.changed();
        Ok(())
    }
}
