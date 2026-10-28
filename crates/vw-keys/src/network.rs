//! Network-record authentication through the same encrypted vault that holds the Mistral key.
//! The vault's item is the key. With "Lock the vault when Vibe Work quits" off, a copy in the computer's credential
//! store (the store that keeps the vault's own key) checks the record while the vault is locked, so a Lock does not
//! drop saved network settings; every Save writes a new key to both, and an open vault makes the copy equal to its
//! item. With the switch on no copy is kept (it is deleted when the switch is turned on, at quit and at start): a
//! locked start leaves the record waiting, and the unlock checks and applies it.
//! Called by host startup and core Settings after native confirmation; no method returns the MAC key.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::PoisonError;

use base64::Engine as _;
use vw_mistral::network::{self, LoadedSettings, NetworkMacKey, NetworkSettings, PreparedSettings};
use vw_vault::items::{ResourceKind, ResourcePermission, VaultResource};
use vw_vault::remember::network_key_account;
use vw_vault::{SecretString, Vault, VaultError, VaultStatus};
use zeroize::Zeroizing;

use crate::service::{blocking, now_ms};
use crate::{KeyError, KeyEvent, KeyService};

const ITEM_ID: &str = "vibe-work-network-mac-key";
const FIELD: &str = "macKey";

/// Why saved network settings wait for a vault unlock before they are checked and used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkWait {
    /// "Lock the vault when Vibe Work quits" is on: no copy of the check key is kept, so every start waits.
    LockAtQuit,
    /// This computer has no copy of the check key yet (an install from before the copy).
    NoCopyYet,
}

/// What the run did with the saved network record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Checked and used, or nothing custom was saved.
    Used,
    /// The record did not pass the check.
    Rejected,
    /// Not used yet: checked at the next vault unlock and, if it passes, used from then on.
    Waits(NetworkWait),
}

/// The run's network record: its file and what was done with it.
pub(crate) struct Network {
    state: State,
    file: Option<PathBuf>,
}

/// The vault item's key, base64 (`None`: no Save stored one).
fn vault_key(vault: &Vault) -> Result<Option<SecretString>, KeyError> {
    let item = vault.read_item(ITEM_ID)?;
    let Some(item) = item else { return Ok(None) };
    let resource = VaultResource::from_item(&item).ok_or(KeyError::Internal)?;
    if resource.permission != ResourcePermission::UserOnly
        || resource.kind != ResourceKind::DeveloperCredential
    {
        return Err(KeyError::Internal);
    }
    let encoded = SecretString::new(
        resource
            .secret_fields
            .get(FIELD)
            .ok_or(KeyError::Internal)?
            .clone(),
    );
    encoded.register_as(vw_secrets::SecretKind::VaultValue);
    Ok(Some(encoded))
}

fn decode(encoded: &SecretString) -> Option<NetworkMacKey> {
    let mut bytes = Zeroizing::new([0_u8; 32]);
    let count = base64::prelude::BASE64_STANDARD
        .decode_slice(encoded.expose(), bytes.as_mut())
        .ok()?;
    (count == bytes.len()).then(|| NetworkMacKey::new(bytes))
}

fn copy_account(vault: &Vault) -> String {
    network_key_account(&vault.location().profile_dir)
}

/// Deletes the credential store's copy (the switch is on). A failed delete is logged.
pub(crate) fn drop_copy(vault: &Vault) {
    if let Err(error) = vault.device_store().delete(&copy_account(vault)) {
        tracing::warn!(%error, "network settings: the key copy was not deleted");
    }
}

/// The switch was turned on (the copy is deleted) or off (an open vault stores the copy now).
pub(crate) fn switch_changed(vault: &Vault, lock_on_quit: bool) {
    if lock_on_quit {
        drop_copy(vault);
    } else {
        unlocked(vault);
    }
}

/// Makes the credential store's copy equal to the vault's key. A failed write is logged: the record is then checked
/// only while the vault is open.
fn keep_copy(vault: &Vault, encoded: &SecretString) {
    let (store, account) = (vault.device_store(), copy_account(vault));
    if matches!(store.get(&account), Ok(Some(copy)) if copy.expose() == encoded.expose()) {
        return;
    }
    if let Err(error) = store.set(&account, encoded) {
        tracing::warn!(%error, "network settings: the key copy was not stored");
        // An older copy must not keep accepting an older record.
        let _ = store.delete(&account);
    }
}

/// The key that checks the record: the vault's while it is open (its copy kept equal when `copy`), the copy while it
/// is locked (only when `copy`). The second value is true when an existing vault was locked and no copy was used.
fn check_key(vault: &Vault, copy: bool) -> (Option<NetworkMacKey>, bool) {
    match vault_key(vault) {
        Ok(encoded) => {
            if let (true, Some(encoded)) = (copy, &encoded) {
                keep_copy(vault, encoded);
            }
            (encoded.as_ref().and_then(decode), false)
        }
        Err(KeyError::Vault(VaultError::Locked)) => {
            let found = if copy {
                vault
                    .device_store()
                    .get(&copy_account(vault))
                    .inspect_err(|error| {
                        tracing::warn!(%error, "network settings: the key copy could not be read");
                    })
                    .ok()
                    .flatten()
            } else {
                None
            };
            if let Some(found) = found {
                found.register_as(vw_secrets::SecretKind::VaultValue);
                return (decode(&found), false);
            }
            // Only an existing vault can hold a key a Save stored; without one a record was never saved here.
            let exists = matches!(vault.status(), Ok(VaultStatus::Locked { .. }));
            (None, exists)
        }
        Err(_) => (None, false),
    }
}

/// After an unlock: the copy follows the vault's key (an install from before the copy gets one here).
pub(crate) fn unlocked(vault: &Vault) {
    if let Ok(Some(encoded)) = vault_key(vault) {
        keep_copy(vault, &encoded);
    }
}

fn rotate_mac_key(vault: &Vault, copy: bool) -> Result<NetworkMacKey, KeyError> {
    let mut bytes = Zeroizing::new([0_u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| KeyError::Internal)?;
    let encoded = SecretString::new(base64::prelude::BASE64_STANDARD.encode(bytes.as_ref()));
    encoded.register_as(vw_secrets::SecretKind::VaultValue);
    let now = now_ms();
    let resource = VaultResource {
        id: ITEM_ID.to_owned(),
        schema_version: vw_vault::items::RESOURCE_SCHEMA_VERSION.to_owned(),
        kind: ResourceKind::DeveloperCredential,
        label: "Network settings authentication (Vibe Work)".to_owned(),
        permission: ResourcePermission::UserOnly,
        public_fields: BTreeMap::new(),
        secret_fields: BTreeMap::from([(FIELD.to_owned(), encoded.expose().to_owned())]),
        custom_fields: Vec::new(),
        created_ms: now,
        updated_ms: now,
        extra: BTreeMap::new(),
    };
    let candidate = resource.to_item()?;
    // Store the new key before returning any record. Interruption must invalidate the previous record.
    vault
        .update_item(ITEM_ID, |_| Some(candidate.clone()))?
        .ok_or(KeyError::Internal)?;
    if copy {
        keep_copy(vault, &encoded);
    }
    Ok(NetworkMacKey::new(bytes))
}

/// The start: checks and applies the record before any key-bearing client exists. With the switch on, any copy
/// is deleted first (a run that did not quit may have left one) and a locked vault leaves the record waiting.
pub(crate) fn launch(vault: &Vault, file: Option<PathBuf>, lock_on_quit: bool) -> Network {
    if lock_on_quit {
        drop_copy(vault);
    }
    let Some(path) = file.as_deref() else {
        return Network {
            state: State::Used,
            file,
        };
    };
    let (key, locked) = check_key(vault, !lock_on_quit);
    let state = match (
        network::load_for_launch(path, key.as_ref()).rejected,
        locked,
    ) {
        (false, _) => State::Used,
        (true, false) => State::Rejected,
        (true, true) if lock_on_quit => State::Waits(NetworkWait::LockAtQuit),
        (true, true) => State::Waits(NetworkWait::NoCopyYet),
    };
    Network { state, file }
}

impl KeyService {
    /// Reads the authenticated network record. Without the vault's key or its copy, custom values are rejected.
    pub async fn read_network_settings(&self, file: PathBuf) -> LoadedSettings {
        let (vault, copy) = (self.vault(), !self.lock_on_quit());
        blocking(move || {
            let (key, _) = check_key(&vault, copy);
            network::read_settings(&file, key.as_ref())
        })
        .await
        .unwrap_or_else(|_| LoadedSettings {
            settings: NetworkSettings::default(),
            rejected: true,
        })
    }

    /// Seals a confirmed candidate after storing a fresh hidden key, including for empty/default Saves.
    /// Older records then fail verification, even if writing the new record is interrupted.
    ///
    /// # Errors
    /// The vault is unavailable or the record is too large.
    pub async fn seal_network_settings(
        &self,
        prepared: PreparedSettings,
    ) -> Result<String, KeyError> {
        let (vault, copy) = (self.vault(), !self.lock_on_quit());
        blocking(move || {
            let key = rotate_mac_key(&vault, copy)?;
            network::seal_settings(&prepared, Some(&key))
                .map_err(|e| KeyError::Settings(e.to_string()))
        })
        .await?
    }

    /// After an unlock: the copy follows the vault's key (switch off), and a record that waited for the vault is
    /// checked now. If it passes it is applied, and the key-bearing clients are built again before the key is read
    /// into the proxy (the caller refreshes the key after this).
    pub(crate) async fn network_unlocked(&self) {
        let (vault, copy) = (self.vault(), !self.lock_on_quit());
        let waiting = {
            let network = self.network.lock().unwrap_or_else(PoisonError::into_inner);
            matches!(network.state, State::Waits(_))
                .then(|| network.file.clone())
                .flatten()
        };
        let checked = blocking(move || {
            if copy {
                unlocked(&vault);
            }
            let file = waiting?;
            let (key, _) = check_key(&vault, copy);
            Some(network::load_for_launch(&file, key.as_ref()).rejected)
        })
        .await
        .ok()
        .flatten();
        let Some(rejected) = checked else { return };
        if !rejected {
            let proxy = self.proxy.lock().await;
            let built = proxy
                .as_ref()
                .map_or(Ok(()), |proxy| proxy.client().rebuild());
            if let Err(error) = built.and_then(|()| self.client.rebuild()) {
                tracing::warn!(%error, "network settings: a client was not built again");
            }
        }
        // The state first: the core words its notice from it.
        self.network
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .state = if rejected {
            State::Rejected
        } else {
            State::Used
        };
        let _ = self
            .events
            .send(KeyEvent::NetworkSettingsChecked(!rejected));
    }

    /// Whether the saved network record is not in use (rejected, or waiting for the vault); the core replays one
    /// notice.
    #[must_use]
    pub fn network_settings_rejected(&self) -> bool {
        self.network_state() != State::Used
    }

    /// Why the saved network record waits for a vault unlock, if it does.
    #[must_use]
    pub fn network_settings_wait(&self) -> Option<NetworkWait> {
        match self.network_state() {
            State::Waits(wait) => Some(wait),
            State::Used | State::Rejected => None,
        }
    }

    fn network_state(&self) -> State {
        self.network
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .state
    }
}
