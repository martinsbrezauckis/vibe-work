//! Vault MAC key lifecycle and the optional credential-store copy. Called by network records and lock settings.
//! The copy authenticates public settings only; proxy sign-ins never use it.

use std::collections::BTreeMap;

use base64::Engine as _;
use vw_mistral::network::NetworkMacKey;
use vw_vault::items::{ResourceKind, ResourcePermission, VaultResource};
use vw_vault::remember::network_key_account;
use vw_vault::{SecretString, Vault, VaultError, VaultStatus};
use zeroize::Zeroizing;

use crate::KeyError;
use crate::service::now_ms;

const ITEM_ID: &str = "vibe-work-network-mac-key";
const FIELD: &str = "macKey";

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
pub(crate) fn check_key(vault: &Vault, copy: bool) -> (Option<NetworkMacKey>, bool) {
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

pub(crate) fn rotate_mac_key(vault: &Vault, copy: bool) -> Result<NetworkMacKey, KeyError> {
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
