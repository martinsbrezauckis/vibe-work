//! Mistral API credentials kept in the vault for Vibe Work.
//!
//! Stored as typed resources of kind `developerCredential`. First setup uses [`RESOURCE_ID`];
//! additional named credentials get fresh identities. The initial label is "Mistral API key (Vibe Work)".
//! Each has `publicFields.provider = "mistral"`,
//! secret field `apiKey`, permission `userOnly` (hidden from every agent surface, incl. Vault's
//! MCP list; can relax it with consent).
//!
//! The intent "a user who stored a secret in Vault never stores it again" is served by
//! [`find_existing`]: before asking for a key, setup lists Mistral credentials already in the vault
//! (typed resources whose provider is Mistral, and ShellX compat items with a Mistral key name) and
//! the user can pick one; its value is then read from that item, not copied.
//!
//! Depends on: `items`, `secret`. Called by: vw-keys.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::items::{ResourceKind, ResourcePermission, VaultItem, VaultResource};
use crate::secret::SecretString;

/// Id of the resource Vibe Work writes.
pub const RESOURCE_ID: &str = "vibe-work-mistral-api-key";
/// Prefix for additional credentials created by this app.
pub const ID_PREFIX: &str = "vibe-work-mistral-";

/// Whether the item identity belongs to the app’s model credentials.
#[must_use]
pub fn is_own(id: &str) -> bool {
    id == RESOURCE_ID
        || id.strip_prefix(ID_PREFIX).is_some_and(|suffix| {
            suffix.len() == 32 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

/// Its label.
pub const RESOURCE_LABEL: &str = "Mistral API key (Vibe Work)";
/// Secret field name.
pub const SECRET_FIELD: &str = "apiKey";

/// ShellX compat key names that hold a Mistral key (ShellX stores a key named `k` as item
/// `kv-<BLAKE3("shellx-vault-kv-v1\0" + k)>`, `resources.rs:241-244` @ the pin).
pub const COMPAT_KEY_NAMES: &[&str] = &[
    "mistral/api-key",
    "providers.mistral.api_key",
    "providers.mistral.apiKey",
    "mistral.api_key",
    "MISTRAL_API_KEY",
];

/// The item id ShellX uses for a compat key name.
#[must_use]
pub fn shellx_compat_item_id(key: &str) -> String {
    let digest = blake3::hash(format!("shellx-vault-kv-v1\0{key}").as_bytes());
    format!("kv-{}", digest.to_hex())
}

/// The resource holding `key`, created now (or keeping `created_ms` of the one it replaces).
#[must_use]
pub fn resource(key: &SecretString, created_ms: Option<i64>, now_ms: i64) -> VaultResource {
    let mut public_fields = BTreeMap::new();
    public_fields.insert("provider".to_owned(), Value::String("mistral".to_owned()));
    public_fields.insert(
        "description".to_owned(),
        Value::String("Used by Vibe Work's local model proxy".to_owned()),
    );
    let mut secret_fields = BTreeMap::new();
    secret_fields.insert(SECRET_FIELD.to_owned(), key.expose().to_owned());
    VaultResource {
        id: RESOURCE_ID.to_owned(),
        schema_version: crate::items::RESOURCE_SCHEMA_VERSION.to_owned(),
        kind: ResourceKind::DeveloperCredential,
        label: RESOURCE_LABEL.to_owned(),
        permission: ResourcePermission::UserOnly,
        public_fields,
        secret_fields,
        custom_fields: Vec::new(),
        created_ms: created_ms.unwrap_or(now_ms),
        updated_ms: now_ms,
        extra: BTreeMap::new(),
    }
}

/// A Mistral credential already in the vault (metadata only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Item id.
    pub item_id: String,
    /// Title shown in the vault apps.
    pub label: String,
    /// `resource` (typed) or `compat` (a ShellX key-value item).
    pub source: &'static str,
}

fn is_mistral_resource(resource: &VaultResource) -> bool {
    resource
        .public_fields
        .get("provider")
        .and_then(Value::as_str)
        .is_some_and(|p| p.eq_ignore_ascii_case("mistral"))
}

/// The key value stored in an item, if it holds one: a typed resource's `apiKey`, `value` (ShellX
/// migration) or its only secret field; a compat item's `password`.
#[must_use]
pub fn key_in_item(item: &VaultItem) -> Option<SecretString> {
    if let Some(resource) = VaultResource::from_item(item) {
        let fields = &resource.secret_fields;
        let value = fields
            .get(SECRET_FIELD)
            .or_else(|| fields.get("value"))
            .or_else(|| {
                (fields.len() == 1)
                    .then(|| fields.values().next())
                    .flatten()
            })?;
        let key = SecretString::new(value.clone());
        key.register_as(vw_secrets::SecretKind::MistralKey);
        return (!key.is_empty()).then_some(key);
    }
    let key = SecretString::new(item.password.clone());
    key.register_as(vw_secrets::SecretKind::MistralKey);
    (!key.is_empty()).then_some(key)
}

/// Mistral credentials in `items`, excluding Vibe Work's own resource.
#[must_use]
pub fn find_existing(items: &[VaultItem]) -> Vec<Candidate> {
    find_all(items)
        .into_iter()
        .filter(|item| item.item_id != RESOURCE_ID)
        .collect()
}

/// A fresh identity for an additional Mistral credential.
#[must_use]
pub fn new_id() -> String {
    format!(
        "{ID_PREFIX}{}",
        hex::encode(vault_core::random_bytes::<16>())
    )
}

/// Every Mistral credential, including the original setup item; metadata only.
#[must_use]
pub fn find_all(items: &[VaultItem]) -> Vec<Candidate> {
    let compat_ids: Vec<String> = COMPAT_KEY_NAMES
        .iter()
        .map(|k| shellx_compat_item_id(k))
        .collect();
    items
        .iter()
        .filter_map(|item| {
            let source = match VaultResource::from_item(item) {
                Some(resource)
                    if resource.kind == ResourceKind::DeveloperCredential
                        && is_mistral_resource(&resource) =>
                {
                    "resource"
                }
                None if compat_ids.contains(&item.id) => "compat",
                Some(_) | None => return None,
            };
            key_in_item(item).map(|_| Candidate {
                item_id: item.id.clone(),
                label: item.title.clone(),
                source,
            })
        })
        .collect()
}
