//! Chrome's owner identity in the OS credential store. Depends on the key service's device store;
//! called only by core setup, launch and Remove. The scalar never enters the vault's files or a client result.
use vw_vault::SecretString;
use zeroize::Zeroizing;

use crate::{KeyError, KeyService, service::blocking};

pub(crate) fn account(path: &std::path::Path) -> String {
    vw_vault::remember::chrome_owner_account(path)
}

fn valid(bytes: &[u8; 32]) -> bool {
    // P-256 scalars are in [1, n); rejecting an invalid draw keeps every accepted scalar equally likely.
    const ORDER: [u8; 32] = [
        0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2, 0xfc, 0x63,
        0x25, 0x51,
    ];
    bytes.iter().any(|b| *b != 0) && bytes < &ORDER
}

fn generate() -> Result<SecretString, KeyError> {
    let mut bytes = Zeroizing::new([0; 32]);
    loop {
        getrandom::fill(bytes.as_mut()).map_err(|_| KeyError::Internal)?;
        if valid(&bytes) {
            let encoded = SecretString::new(hex::encode(bytes.as_ref()));
            encoded.register_as(vw_secrets::SecretKind::VaultValue);
            return Ok(encoded);
        }
    }
}

impl KeyService {
    /// Stores a new owner scalar. Only setup after the person's click calls this.
    ///
    /// # Errors
    /// The OS credential store cannot save the identity.
    pub async fn create_chrome_owner(&self) -> Result<(), KeyError> {
        let vault = self.vault();
        let account = self.chrome_owner_account.clone();
        blocking(move || {
            vault.device_store().set(&account, &generate()?)?;
            Ok(())
        })
        .await?
    }

    /// Reads the owner scalar for the private helper pipe. Never exposed by the command API.
    ///
    /// # Errors
    /// The credential store cannot read the identity.
    pub async fn chrome_owner(&self) -> Result<Option<SecretString>, KeyError> {
        let vault = self.vault();
        let account = self.chrome_owner_account.clone();
        blocking(move || {
            let secret = vault.device_store().get(&account)?;
            if let Some(secret) = &secret {
                secret.register_as(vw_secrets::SecretKind::VaultValue);
            }
            Ok(secret)
        })
        .await?
    }

    /// Removes the owner identity, including while the vault is locked.
    ///
    /// # Errors
    /// The OS credential store cannot delete the identity.
    pub async fn delete_chrome_owner(&self) -> Result<(), KeyError> {
        let vault = self.vault();
        let account = self.chrome_owner_account.clone();
        blocking(move || {
            vault.device_store().delete(&account)?;
            vault.device_store().delete(&format!("{account}-policy"))?;
            Ok(())
        })
        .await?
    }
}

impl KeyService {
    /// Reads Chrome's trusted restrictions independently of any agent-writable state file.
    ///
    /// # Errors
    /// The credential store is unavailable or the stored policy is malformed.
    pub async fn chrome_restrictions(
        &self,
    ) -> Result<std::collections::BTreeMap<String, String>, KeyError> {
        let vault = self.vault();
        let account = format!("{}-policy", self.chrome_owner_account);
        blocking(move || {
            let Some(encoded) = vault.device_store().get(&account)? else {
                return Ok(std::collections::BTreeMap::new());
            };
            let rows: std::collections::BTreeMap<String, String> =
                serde_json::from_str(encoded.expose()).map_err(|_| KeyError::Internal)?;
            if rows
                .values()
                .any(|value| value != "ask" && value != "never")
            {
                return Err(KeyError::Internal);
            }
            Ok(rows)
        })
        .await?
    }

    /// Persists a trusted extension lowering or a person-confirmed replacement.
    ///
    /// # Errors
    /// The OS credential store cannot save the policy.
    pub async fn save_chrome_restrictions(
        &self,
        rows: std::collections::BTreeMap<String, String>,
    ) -> Result<(), KeyError> {
        let vault = self.vault();
        let account = format!("{}-policy", self.chrome_owner_account);
        blocking(move || {
            let encoded =
                SecretString::new(serde_json::to_string(&rows).map_err(|_| KeyError::Internal)?);
            vault
                .device_store()
                .set(&account, &encoded)
                .map_err(KeyError::from)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_bounds_and_generated_encoding() {
        assert!(!valid(&[0; 32]));
        assert!(!valid(&[255; 32]));
        assert!(valid(&[1; 32]));
        let secret = generate().unwrap();
        assert_eq!(secret.expose().len(), 64);
        assert!(
            secret
                .expose()
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }
}
