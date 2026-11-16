//! Moves the app's OS credential-store entries from ShellX's service name to Vibe Work's own, without loss.
//!
//! Every entry the app keeps (the remembered device of its own vaults, the Chrome Control owner identity, the
//! network-settings check key) lives under [`KEYRING_SERVICE`]. Entries an older version wrote under
//! [`SHELLX_KEYRING_SERVICE`] move on first use: read the new name; when absent read the old one, write it under
//! the new name, read it back, and only when the copy matches delete the old entry. A stop at any step leaves a
//! readable entry: the old one until the copy is proven, the copy afterwards. A copy that exists next to an old
//! entry (a stop between the copy and the deletion) wins, and the old entry is deleted then.
//!
//! The one exception is the remembered device of ShellX's own shared vault: ShellX reads and writes that entry
//! under its own name, so moving it would make each app's passphrase unlock break the other's remembered unlock.
//! An account marked shared stays on the old name and never moves.
//!
//! Depends on: `remember` (the names, [`DeviceSecretStore`]). Called by: [`crate::remember::OsDeviceStore`].
//! Invariant: an old entry is deleted only after the new name returned the same secret, or when the new name
//! already held one at start.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

use crate::error::VaultError;
use crate::remember::{DeviceSecretStore, KEYRING_SERVICE, SHELLX_KEYRING_SERVICE};
use crate::secret::SecretString;

/// One OS credential store addressed by service name (the platform store in the product, a fake in tests).
pub(crate) trait NamedStore: Send + Sync {
    /// The secret stored under `service`/`account`, if any.
    fn read(&self, service: &str, account: &str) -> Result<Option<SecretString>, VaultError>;
    /// Stores or replaces the secret under `service`/`account`.
    fn write(&self, service: &str, account: &str, secret: &SecretString) -> Result<(), VaultError>;
    /// Deletes the entry under `service`/`account` (absent is fine).
    fn remove(&self, service: &str, account: &str) -> Result<(), VaultError>;
}

/// A [`DeviceSecretStore`] over a [`NamedStore`] that keeps everything under [`KEYRING_SERVICE`] and moves old
/// entries there (module doc).
pub(crate) struct Renamed<S> {
    store: S,
    /// The remembered-device account of ShellX's shared vault, which keeps ShellX's service name.
    shared: Option<String>,
    /// Accounts whose old entry is known to be gone in this run (no further lookup of the old name).
    finished: Mutex<HashSet<String>>,
}

impl<S: NamedStore> Renamed<S> {
    pub(crate) fn new(store: S) -> Self {
        Self {
            store,
            shared: None,
            finished: Mutex::default(),
        }
    }

    /// Keeps `account` (ShellX's shared vault's remembered device) under ShellX's service name.
    pub(crate) fn sharing(mut self, account: String) -> Self {
        self.shared = Some(account);
        self
    }

    #[cfg(test)]
    pub(crate) fn inner(&self) -> &S {
        &self.store
    }

    fn is_shared(&self, account: &str) -> bool {
        self.shared.as_deref() == Some(account)
    }

    fn is_finished(&self, account: &str) -> bool {
        self.finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(account)
    }

    fn finish(&self, account: &str) {
        self.finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account.to_owned());
    }

    /// Deletes the old entry once the new name holds the secret. A failure keeps it for the next start, which
    /// tries again; the secret is already readable under the new name, so nothing waits on it.
    fn retire_old(&self, account: &str) {
        if !self.is_finished(account) && self.store.remove(SHELLX_KEYRING_SERVICE, account).is_ok()
        {
            self.finish(account);
        }
    }

    /// Copies an old secret to the new name and deletes the old entry only once the copy reads back the same.
    /// Any failure leaves the old entry as it was (and removes an unproven copy), so the next start moves it.
    fn move_old(&self, account: &str, old: &SecretString) {
        if self.store.write(KEYRING_SERVICE, account, old).is_err() {
            return;
        }
        match self.store.read(KEYRING_SERVICE, account) {
            Ok(Some(copy)) if copy.expose() == old.expose() => self.retire_old(account),
            _ => {
                let _ = self.store.remove(KEYRING_SERVICE, account);
            }
        }
    }
}

impl<S: NamedStore> DeviceSecretStore for Renamed<S> {
    fn get(&self, account: &str) -> Result<Option<SecretString>, VaultError> {
        if self.is_shared(account) {
            return self.store.read(SHELLX_KEYRING_SERVICE, account);
        }
        if let Some(found) = self.store.read(KEYRING_SERVICE, account)? {
            self.retire_old(account);
            return Ok(Some(found));
        }
        if self.is_finished(account) {
            return Ok(None);
        }
        let Some(old) = self.store.read(SHELLX_KEYRING_SERVICE, account)? else {
            self.finish(account);
            return Ok(None);
        };
        // The old secret unlocks now whatever the move does; a failed move is retried at the next start.
        self.move_old(account, &old);
        Ok(Some(old))
    }

    fn set(&self, account: &str, secret: &SecretString) -> Result<(), VaultError> {
        if self.is_shared(account) {
            return self.store.write(SHELLX_KEYRING_SERVICE, account, secret);
        }
        self.store.write(KEYRING_SERVICE, account, secret)?;
        // An old entry would otherwise outlive the newer secret; the new name always wins, so this is tidying.
        self.retire_old(account);
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), VaultError> {
        if self.is_shared(account) {
            return self.store.remove(SHELLX_KEYRING_SERVICE, account);
        }
        // Old first: were the old entry left after the new one is gone, the next start would move it back and
        // remember what the person turned off.
        self.store.remove(SHELLX_KEYRING_SERVICE, account)?;
        self.finish(account);
        self.store.remove(KEYRING_SERVICE, account)
    }
}

#[cfg(test)]
#[path = "remember_rename_tests.rs"]
mod tests;
