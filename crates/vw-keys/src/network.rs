//! Network-record authentication through the same encrypted vault that holds the Mistral key.
//! Called by host startup and core Settings after native confirmation; no method returns the MAC key.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use vw_mistral::network::{self, LoadedSettings, NetworkMacKey, NetworkSettings, PreparedSettings};
use vw_vault::items::{ResourceKind, ResourcePermission, VaultResource};
use vw_vault::{SecretString, Vault};
use zeroize::Zeroizing;

use crate::service::{blocking, now_ms};
use crate::{KeyError, KeyService};

const ITEM_ID: &str = "vibe-work-network-mac-key";
const FIELD: &str = "macKey";

fn mac_key(vault: &Vault) -> Result<Option<NetworkMacKey>, KeyError> {
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
    let mut bytes = Zeroizing::new([0_u8; 32]);
    let count = base64::prelude::BASE64_STANDARD
        .decode_slice(encoded.expose(), bytes.as_mut())
        .map_err(|_| KeyError::Internal)?;
    if count != bytes.len() {
        return Err(KeyError::Internal);
    }
    Ok(Some(NetworkMacKey::new(bytes)))
}

fn rotate_mac_key(vault: &Vault) -> Result<NetworkMacKey, KeyError> {
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
    Ok(NetworkMacKey::new(bytes))
}

pub(crate) fn load_for_launch(vault: &Vault, file: &Path) -> LoadedSettings {
    let key = mac_key(vault).ok().flatten();
    network::load_for_launch(file, key.as_ref())
}

impl KeyService {
    /// Reads the authenticated network record. A locked or unavailable vault rejects custom values.
    pub async fn read_network_settings(&self, file: PathBuf) -> LoadedSettings {
        let vault = self.vault();
        blocking(move || {
            let key = mac_key(&vault).ok().flatten();
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
        let vault = self.vault();
        blocking(move || {
            let key = rotate_mac_key(&vault)?;
            network::seal_settings(&prepared, Some(&key))
                .map_err(|e| KeyError::Settings(e.to_string()))
        })
        .await?
    }

    /// Whether launch rejected the stored network record; the core replays one security notice.
    #[must_use]
    pub fn network_settings_rejected(&self) -> bool {
        self.network_rejected
    }
}
