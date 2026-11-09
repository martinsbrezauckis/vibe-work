//! Password verification and explicit system-unlock enrollment. Depends on the vault facade and ShellX keyfiles;
//! called by the key service's settings transition. Neither action opens a locked session or rewrites vault items.
use super::{LocalVault, Vault};
use crate::{SecretString, VaultError};
use vault_core::{Keyfile, MasterKey};

pub(super) fn keyfile_from_json(raw: &str) -> Result<Keyfile, VaultError> {
    if raw.len() > 64 * 1024 {
        return Err(VaultError::Corrupt("keyfile too large".to_owned()));
    }
    serde_json::from_str(raw).map_err(|_| VaultError::Corrupt("keyfile is not valid".to_owned()))
}

pub(super) fn unlock_keyfile(
    keyfile: &Keyfile,
    secret: &SecretString,
) -> Result<MasterKey, VaultError> {
    keyfile
        .unlock(secret.expose())
        .map_err(|error| match error {
            vault_core::KeysError::Unlock => VaultError::WrongPassphrase,
            other => VaultError::Corrupt(other.to_string()),
        })
}

impl Vault {
    /// Verifies the primary vault password without changing the session or remembered keys.
    /// # Errors
    /// Wrong password or an unavailable/corrupt vault.
    pub fn verify_password(&self, passphrase: &SecretString) -> Result<(), VaultError> {
        self.verify_password_choice(passphrase, None)
    }

    /// Verifies the password of an explicit first-run choice without selecting it.
    /// # Errors
    /// Wrong password, invalid choice or an unavailable vault.
    pub fn verify_password_choice(
        &self,
        passphrase: &SecretString,
        choice: Option<crate::VaultKind>,
    ) -> Result<(), VaultError> {
        let raw = match self.pick(choice)? {
            LocalVault::Shellx { profile, .. } => profile
                .keyfile_json
                .ok_or_else(|| VaultError::Corrupt("the profile has no keyfile".to_owned()))?,
            LocalVault::Desktop { keyfile_path, .. } => std::fs::read_to_string(&keyfile_path)
                .map_err(|e| VaultError::io("read the keyfile", &keyfile_path, e))?,
        };
        unlock_keyfile(&keyfile_from_json(&raw)?, passphrase)?;
        Ok(())
    }

    /// Explicitly enrolls system unlock; unlike ordinary unlock, a store failure is returned to settings.
    /// # Errors
    /// Wrong password, unsupported layout, or a failed store/profile write.
    pub fn keep_unlock_key(&self, passphrase: &SecretString) -> Result<(), VaultError> {
        let Some((mut profile, dir)) = self.kept_key_profile()? else {
            return Err(VaultError::Remember(
                "this vault has no remembered device".to_owned(),
            ));
        };
        let keyfile = keyfile_from_json(
            profile
                .keyfile_json
                .as_deref()
                .ok_or_else(|| VaultError::Corrupt("the profile has no keyfile".to_owned()))?,
        )?;
        self.remember_into(&mut profile, &keyfile, passphrase, &dir)?;
        if let Err(error) = profile.write(&dir) {
            let _ = self.config.device_store.delete(&self.device_account(&dir));
            return Err(error);
        }
        if let Some(session) = self.session().as_mut() {
            session.remembered = true;
        }
        self.note(None);
        Ok(())
    }
}
