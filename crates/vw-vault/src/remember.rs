//! "Remember this device", ShellX's scheme (the standalone Vault app has none), reproduced
//! exactly so ShellX and Vibe Work unlock each other's vault:
//! - a 32-byte random device secret, hex (`backend.rs:2360-2362` @ e38c2b5c), is stored in the OS
//!   credential store under service `shellx-vault`, account
//!   `remembered-device-v1-<BLAKE3 hex of the profile folder path string>` (`:959-963,2349-2350`);
//! - `rememberedKeyfileJson` in `profile.json` is the keyfile rewrapped under that secret
//!   (`Keyfile::rewrap(passphrase, device_secret)`, `:2364-2373`), so the secret unlocks the vault
//!   like a passphrase;
//! - a new secret is made on every passphrase unlock that keeps remembering, and turning it off
//!   deletes the entry (`:996-1024`).
//!
//! The OS entry naming of `keyring` 2.3.3 (ShellX) and `keyring-core` 1.0 stores is the same:
//! Windows target `<user>.<service>`, macOS login-keychain generic password (service, account),
//! Linux Secret Service attributes `service` + `username`.
//!
//! Residual risk (copy): on Linux and Windows any program running as the same user
//! can read the entry (no per-app access control there); on macOS another app gets a prompt.
//!
//! Depends on: keyring-core and the OS store crate, blake3, vault-core. Called by: `vault`.
//! Invariant: secrets cross this module only as [`SecretString`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use vault_core::Keyfile;

use crate::error::VaultError;
use crate::secret::SecretString;

/// OS credential store service name (ShellX's).
pub const KEYRING_SERVICE: &str = "shellx-vault";
/// Account prefix (ShellX's).
pub const ACCOUNT_PREFIX: &str = "remembered-device-v1";

/// The account for a profile folder, exactly as ShellX derives it.
#[must_use]
pub fn account_for(profile_dir: &Path) -> String {
    let raw = profile_dir.to_string_lossy();
    format!("{ACCOUNT_PREFIX}-{}", blake3::hash(raw.as_bytes()).to_hex())
}

/// A fresh device secret: 32 random bytes, hex.
#[must_use]
pub fn new_device_secret() -> SecretString {
    let secret = SecretString::new(hex::encode(vault_core::random_bytes::<32>()));
    secret.register_as(vw_secrets::SecretKind::VaultValue);
    secret
}

/// The keyfile rewrapped under the device secret, as pretty JSON (ShellX's format).
///
/// # Errors
/// [`VaultError::WrongPassphrase`] when `passphrase` does not open `primary`.
pub fn remembered_keyfile_json(
    primary: &Keyfile,
    passphrase: &SecretString,
    device_secret: &SecretString,
) -> Result<String, VaultError> {
    let rewrapped = primary
        .rewrap(passphrase.expose(), device_secret.expose())
        .map_err(|_| VaultError::WrongPassphrase)?;
    serde_json::to_string_pretty(&rewrapped).map_err(|e| VaultError::Corrupt(e.to_string()))
}

/// Where device secrets live. The product uses [`OsDeviceStore`]; tests and runs over SSH use
/// [`MemoryDeviceStore`] (Windows `CredRead` fails with 1312 without an interactive logon).
pub trait DeviceSecretStore: Send + Sync {
    /// The secret for `account`, if any.
    ///
    /// # Errors
    /// [`VaultError::Remember`] when the store fails.
    fn get(&self, account: &str) -> Result<Option<SecretString>, VaultError>;
    /// Stores or replaces the secret.
    ///
    /// # Errors
    /// [`VaultError::Remember`].
    fn set(&self, account: &str, secret: &SecretString) -> Result<(), VaultError>;
    /// Deletes it (absent is fine).
    ///
    /// # Errors
    /// [`VaultError::Remember`].
    fn delete(&self, account: &str) -> Result<(), VaultError>;
}

/// In-memory store (tests, SSH runs).
#[derive(Default)]
pub struct MemoryDeviceStore(Mutex<HashMap<String, SecretString>>);

impl DeviceSecretStore for MemoryDeviceStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, VaultError> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .cloned())
    }

    fn set(&self, account: &str, secret: &SecretString) -> Result<(), VaultError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account.to_owned(), secret.clone());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), VaultError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account);
        Ok(())
    }
}

/// The OS credential store (Windows Credential Manager, macOS login keychain, Linux Secret Service).
pub struct OsDeviceStore {
    store: Arc<keyring_core::api::CredentialStore>,
}

fn remember_error(error: &keyring_core::Error) -> VaultError {
    VaultError::Remember(error.to_string())
}

/// The platform's credential store (shared with vw-keys, which reads Vibe's own key entry for
/// the import offer).
///
/// # Errors
/// [`VaultError::Remember`] when the platform has no usable store (e.g. no Secret Service session
/// on Linux).
pub fn os_credential_store() -> Result<Arc<keyring_core::api::CredentialStore>, VaultError> {
    #[cfg(target_os = "linux")]
    let store: Arc<keyring_core::api::CredentialStore> =
        zbus_secret_service_keyring_store::Store::new().map_err(|e| remember_error(&e))?;
    #[cfg(windows)]
    let store: Arc<keyring_core::api::CredentialStore> =
        windows_native_keyring_store::Store::new().map_err(|e| remember_error(&e))?;
    #[cfg(target_os = "macos")]
    let store: Arc<keyring_core::api::CredentialStore> =
        apple_native_keyring_store::keychain::Store::new().map_err(|e| remember_error(&e))?;
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    return Err(VaultError::Remember(
        "no credential store on this platform".to_owned(),
    ));
    #[cfg(any(target_os = "linux", windows, target_os = "macos"))]
    Ok(store)
}

impl OsDeviceStore {
    /// Opens the platform store.
    ///
    /// # Errors
    /// [`VaultError::Remember`] when the platform has no usable store (e.g. no Secret Service
    /// session on Linux).
    pub fn new() -> Result<Self, VaultError> {
        Ok(Self {
            store: os_credential_store()?,
        })
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry, VaultError> {
        self.store
            .build(KEYRING_SERVICE, account, None)
            .map_err(|e| remember_error(&e))
    }
}

impl DeviceSecretStore for OsDeviceStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, VaultError> {
        match self.entry(account)?.get_password() {
            Ok(secret) => {
                let secret = SecretString::new(secret);
                secret.register_as(vw_secrets::SecretKind::VaultValue);
                Ok(Some(secret))
            }
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(remember_error(&error)),
        }
    }

    fn set(&self, account: &str, secret: &SecretString) -> Result<(), VaultError> {
        self.entry(account)?
            .set_password(secret.expose())
            .map_err(|e| remember_error(&e))
    }

    fn delete(&self, account: &str) -> Result<(), VaultError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(remember_error(&error)),
        }
    }
}
