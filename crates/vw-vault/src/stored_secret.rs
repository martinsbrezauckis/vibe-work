//! Versioned TPM envelope for Windows credential-store values. Uses vault-core's AEAD and the platform CNG edge.
//! Called only by `OsDeviceStore`. The account is authenticated; lost keys and corrupt envelopes fail closed.
//! Depends on: vault-core authenticated encryption and platform CNG.
//! Called by: the vault OS device store.
use crate::{SecretString, VaultError};
use base64::{Engine as _, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const PREFIX: &str = "vw-tpm-v1:";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    key: String,
    body: String,
}
fn error() -> VaultError {
    VaultError::Remember("the stored TPM key could not be opened".to_owned())
}

fn seal(
    account: &str,
    value: &SecretString,
    key: &[u8; 32],
    wrapped: &[u8],
) -> Result<SecretString, VaultError> {
    let envelope = Envelope {
        key: BASE64_STANDARD.encode(wrapped),
        body: BASE64_STANDARD.encode(vault_core::crypto::seal(
            key,
            account.as_bytes(),
            value.expose().as_bytes(),
        )),
    };
    Ok(SecretString::new(format!(
        "{PREFIX}{}",
        serde_json::to_string(&envelope).map_err(|_| error())?
    )))
}
fn open(
    account: &str,
    value: &str,
    unwrap: impl FnOnce(&[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError>,
) -> Result<SecretString, VaultError> {
    let envelope: Envelope =
        serde_json::from_str(value.strip_prefix(PREFIX).ok_or_else(error)?).map_err(|_| error())?;
    let wrapped = BASE64_STANDARD.decode(envelope.key).map_err(|_| error())?;
    let key = unwrap(&wrapped)?;
    let key: &[u8; 32] = key.as_slice().try_into().map_err(|_| error())?;
    let body = BASE64_STANDARD.decode(envelope.body).map_err(|_| error())?;
    let bytes = Zeroizing::new(
        vault_core::crypto::open(key, account.as_bytes(), &body).map_err(|_| error())?,
    );
    let text = std::str::from_utf8(&bytes).map_err(|_| error())?;
    Ok(SecretString::new(text))
}

#[cfg(windows)]
pub(super) fn protect(account: &str, value: &SecretString) -> Result<SecretString, VaultError> {
    let key = Zeroizing::new(vault_core::random_bytes::<32>());
    let Some(wrapped) = vw_platform::vault_tpm::wrap(&key).map_err(VaultError::Remember)? else {
        // No usable TPM: preserve the existing Windows sign-in store, with no claim of app isolation.
        return Ok(value.clone());
    };
    // Prove this key can be recovered before replacing a legacy entry. Reset/missing TPM never destroys the old value.
    let checked = vw_platform::vault_tpm::unwrap(&wrapped).map_err(VaultError::Remember)?;
    if checked.as_slice() != key.as_slice() {
        return Err(error());
    }
    seal(account, value, &key, &wrapped)
}

#[cfg(windows)]
pub(super) fn unprotect(
    account: &str,
    value: &SecretString,
) -> Result<Option<SecretString>, VaultError> {
    if !value.expose().starts_with(PREFIX) {
        return Ok(None);
    }
    open(account, value.expose(), |wrapped| {
        vw_platform::vault_tpm::unwrap(wrapped).map_err(VaultError::Remember)
    })
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn envelope_hides_values_and_binds_the_account() {
        let value = SecretString::new("synthetic-vault-or-chrome-owner-secret");
        let encoded = seal("account-a", &value, &[7; 32], b"opaque-tpm-key").unwrap();
        assert!(!encoded.expose().contains(value.expose()));
        let read = |wrapped: &[u8]| {
            assert_eq!(wrapped, b"opaque-tpm-key");
            Ok(Zeroizing::new(vec![7; 32]))
        };
        assert_eq!(
            open("account-a", encoded.expose(), read).unwrap().expose(),
            value.expose()
        );
        assert!(open("account-b", encoded.expose(), read).is_err());
        assert!(open("account-a", encoded.expose(), |_| Err(error())).is_err());
        assert!(
            open("account-a", encoded.expose(), |_| Ok(Zeroizing::new(
                vec![8; 32]
            )))
            .is_err()
        );
        assert!(open("account-a", "vw-tpm-v1:{broken", read).is_err());
    }
}
