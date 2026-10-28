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
//! Linux: the Secret Service is a D-Bus service, and D-Bus can start one on request that then never answers. Every
//! call to it therefore runs with a deadline ([`bounded`]): opening the store [`CONNECT_TIMEOUT`] (no prompt can be
//! pending there), a read, write or delete [`CALL_TIMEOUT`] (the keyring may be showing its own unlock prompt). Past
//! the deadline the call fails with "the system keyring did not answer" and the app goes on without it.
//!
//! Depends on: keyring-core and the OS store crate, blake3, vault-core. Called by: `vault`, vw-keys (`existing`).
//! Invariant: secrets cross this module only as [`SecretString`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use vault_core::Keyfile;

use crate::error::VaultError;
use crate::secret::SecretString;

/// Longest wait for the OS store to open (Linux: the Secret Service's session; nothing asks the user there).
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Longest wait for one read, write or delete (Linux: it may wait for the keyring's own unlock prompt).
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// What a call that ran out of time says.
pub const NO_ANSWER: &str = "the system keyring did not answer";
/// Why the app's fallback store keeps nothing (no OS credential store could be opened).
pub const NO_STORE: &str = "this computer has no system keyring";

/// Runs `call` with a deadline on Linux (module doc); elsewhere it runs as it is. A call past the deadline keeps
/// its own thread until the store answers or the app quits; the caller gets [`NO_ANSWER`] at once.
///
/// # Errors
/// [`NO_ANSWER`] past `timeout`, or `call`'s own error.
pub fn bounded<T: Send + 'static>(
    timeout: std::time::Duration,
    call: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    if !cfg!(target_os = "linux") {
        return call();
    }
    let (sender, answer) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("os-credential-store".to_owned())
        .spawn(move || {
            let _ = sender.send(call());
        })
        .map_err(|error| error.to_string())?;
    answer
        .recv_timeout(timeout)
        .unwrap_or_else(|_| Err(NO_ANSWER.to_owned()))
}

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

/// Account prefix of Vibe Work's copy of its network-settings check key (an entry ShellX does not read).
pub const NETWORK_KEY_PREFIX: &str = "vibe-work-network-key-v1";

/// The account of that copy for a profile folder, derived as [`account_for`] derives the device secret's.
#[must_use]
pub fn network_key_account(profile_dir: &Path) -> String {
    let raw = profile_dir.to_string_lossy();
    format!(
        "{NETWORK_KEY_PREFIX}-{}",
        blake3::hash(raw.as_bytes()).to_hex()
    )
}

/// Account prefix for the app's long-lived Chrome owner identity and trusted policy.
pub const CHROME_OWNER_PREFIX: &str = "vibe-work-chrome-owner-v1";

/// The owner account for one app settings path, independent of the selected vault.
#[must_use]
pub fn chrome_owner_account(settings: &Path) -> String {
    format!(
        "{CHROME_OWNER_PREFIX}-{}",
        blake3::hash(settings.to_string_lossy().as_bytes()).to_hex()
    )
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
pub struct MemoryDeviceStore(Mutex<HashMap<String, SecretString>>, bool);

impl MemoryDeviceStore {
    /// The app's fallback when the OS store could not be opened: it keeps nothing. Every write is refused with the
    /// reason, so nothing reports itself kept across starts: the vault stays "not remembered" (the setup result says
    /// the password is needed at the next start) and Chrome Control's long-lived identity fails instead of pretending.
    #[must_use]
    pub fn temporary() -> Self {
        Self(Mutex::default(), true)
    }
}

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
        if self.1 {
            return Err(VaultError::Remember(
                if account.starts_with(CHROME_OWNER_PREFIX) {
                    "Chrome Control needs the OS credential store"
                } else {
                    NO_STORE
                }
                .to_owned(),
            ));
        }
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
    // Linux: a store that did not answer once is not waited for again in this run (the app opens it twice at start).
    #[cfg(target_os = "linux")]
    static SILENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    #[cfg(target_os = "linux")]
    if SILENT.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(VaultError::Remember(NO_ANSWER.to_owned()));
    }
    #[cfg(target_os = "linux")]
    let store: Arc<keyring_core::api::CredentialStore> = bounded(CONNECT_TIMEOUT, || {
        zbus_secret_service_keyring_store::Store::new()
            .map(|store| -> Arc<keyring_core::api::CredentialStore> { store })
            .map_err(|e| e.to_string())
    })
    .map_err(|error| {
        if error == NO_ANSWER {
            SILENT.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        VaultError::Remember(error)
    })?;
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
        let entry = self.entry(account)?;
        let found = bounded(CALL_TIMEOUT, move || match entry.get_password() {
            Ok(secret) => Ok(Some(SecretString::new(secret))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        })
        .map_err(VaultError::Remember)?;
        if let Some(secret) = &found {
            secret.register_as(vw_secrets::SecretKind::VaultValue);
        }
        Ok(found)
    }

    fn set(&self, account: &str, secret: &SecretString) -> Result<(), VaultError> {
        let entry = self.entry(account)?;
        let secret = secret.clone();
        bounded(CALL_TIMEOUT, move || {
            entry
                .set_password(secret.expose())
                .map_err(|e| e.to_string())
        })
        .map_err(VaultError::Remember)
    }

    fn delete(&self, account: &str) -> Result<(), VaultError> {
        let entry = self.entry(account)?;
        bounded(CALL_TIMEOUT, move || match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        })
        .map_err(VaultError::Remember)
    }
}

#[cfg(test)]
mod owner_store_tests {
    use super::*;
    #[test]
    fn fallback_keeps_nothing_that_should_outlive_the_run() {
        let store = MemoryDeviceStore::temporary();
        let secret = SecretString::new("fixture value");
        let owner = chrome_owner_account(Path::new("scratch/settings.json"));
        assert!(store.set(&owner, &secret).is_err());
        assert!(store.set(&format!("{owner}-policy"), &secret).is_err());
        assert!(store.get(&owner).unwrap().is_none());
        // A remembered device would be reported as kept while the next start finds nothing.
        let device = format!("{ACCOUNT_PREFIX}-fixture");
        assert_eq!(
            store.set(&device, &secret).unwrap_err().to_string(),
            format!("this device cannot be remembered: {NO_STORE}")
        );
        assert!(store.get(&device).unwrap().is_none());
        store.delete(&device).unwrap();
    }
}
