//! Project secrets: API keys and tokens the agent may use in a command through the broker, never as a
//! value.
//!
//! Format: the typed resource every Vault app reads (`items::VaultResource`, upstream
//! `vault-broker/src/resources.rs`). Vibe Work writes kind `developerCredential`, permission `visibleAsk` (Vault's
//! default: agents see the name, every use needs approval), id `vibe-work-secret-<16 hex>`, one secret field
//! [`VALUE_FIELD`], public fields `createdBy = "vibe-work"` and optionally `sourceOrigin` (the page a key was captured
//! from). Which resources the agent may name: [`agent_visible`] (kind `developerCredential` or `secret`, not
//! `userOnly`, with at least one non-empty secret field). The Mistral key is `userOnly` and never listed.
//!
//! an optional [`SecretLabel`] ("mcp": a key for an MCP server) is kept as one more public field
//! `label`. It is a SUGGESTION only (skill text, hints, picker order): no function in this module or its callers may
//! branch on it for an access decision, and [`value_of`] / [`agent_visible`] treat labelled and plain keys alike.
//!
//! Depends on: `items`, `secret`, vault-core (random ids), vw-secrets. Called by: vw-keys (`secrets`).
//! Invariants:
//! - [`ProjectSecret`] (what lists return) carries metadata only; a value leaves this module only as a
//!   [`SecretString`] from [`value_of`], registered as a stored secret first.
//! - Only resources this module created ([`is_own`]) are replaced or removed by Vibe Work.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::items::{ResourceKind, ResourcePermission, VaultItem, VaultResource};
use crate::secret::SecretString;

/// Id prefix of the secrets Vibe Work creates.
pub const ID_PREFIX: &str = "vibe-work-secret-";
/// The secret field Vibe Work writes.
pub const VALUE_FIELD: &str = "value";
/// Longest name (label) of a secret, in characters.
pub const MAX_NAME: usize = 80;
/// Longest value, in bytes (ShellX's capture limit).
pub const MAX_VALUE: usize = 4096;

/// The public field that holds the label.
pub const LABEL_FIELD: &str = "label";

/// What a key is for. Only a suggestion: it never changes storage, grants, consent or revoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretLabel {
    /// A key for an MCP server.
    Mcp,
}

impl SecretLabel {
    /// The wire / stored word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
        }
    }

    /// Parses the wire word; an unknown word is no label (a plain key).
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "mcp" => Some(Self::Mcp),
            _ => None,
        }
    }
}

/// A secret the agent may name (metadata only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSecret {
    /// Resource id.
    pub id: String,
    /// The name people and agents use.
    pub name: String,
    /// `developerCredential` or `secret`.
    pub kind: &'static str,
    /// Secret field names (never values).
    pub fields: Vec<String>,
    /// True for a secret Vibe Work created (it may replace or remove it).
    pub own: bool,
    /// The page origin a captured key came from.
    pub source_origin: Option<String>,
    /// What the key is for (a suggestion only), `None` for a plain key.
    pub label: Option<SecretLabel>,
    /// Last change, ms.
    pub updated_ms: i64,
}

/// True when `id` is a secret Vibe Work created.
#[must_use]
pub fn is_own(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX)
        .is_some_and(|rest| rest.len() == 16 && rest.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A new random id for a secret.
#[must_use]
pub fn new_id() -> String {
    format!(
        "{ID_PREFIX}{}",
        hex::encode(vault_core::random_bytes::<8>())
    )
}

/// A name a secret may have: 1 to [`MAX_NAME`] characters after trimming, no control characters, no characters
/// that hide or reorder text (`vw_gate`-style display safety is applied again where it is shown).
#[must_use]
pub fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && !name.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
        })
}

fn kind_name(kind: ResourceKind) -> Option<&'static str> {
    match kind {
        ResourceKind::DeveloperCredential => Some("developerCredential"),
        ResourceKind::Secret => Some("secret"),
        _ => None,
    }
}

/// The metadata of `item` when the agent may name it ([`ProjectSecret`]), else `None`.
#[must_use]
pub fn agent_visible(item: &VaultItem) -> Option<ProjectSecret> {
    let resource = VaultResource::from_item(item)?;
    if resource.permission == ResourcePermission::UserOnly {
        return None;
    }
    let kind = kind_name(resource.kind)?;
    let fields: Vec<String> = resource
        .secret_fields
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, _)| k.clone())
        .collect();
    if fields.is_empty() {
        return None;
    }
    Some(ProjectSecret {
        own: is_own(&resource.id),
        source_origin: resource
            .public_fields
            .get("sourceOrigin")
            .and_then(Value::as_str)
            .map(str::to_owned),
        label: resource
            .public_fields
            .get(LABEL_FIELD)
            .and_then(Value::as_str)
            .and_then(SecretLabel::parse),
        id: resource.id,
        name: resource.label,
        kind,
        fields,
        updated_ms: resource.updated_ms,
    })
}

/// The value of `field` (default: [`VALUE_FIELD`], else the only field) of an agent-visible secret, registered as a
/// stored vault value. `None` when the item is not agent-visible or has no such non-empty field.
#[must_use]
pub fn value_of(item: &VaultItem, field: Option<&str>) -> Option<SecretString> {
    agent_visible(item)?;
    let resource = VaultResource::from_item(item)?;
    let fields = &resource.secret_fields;
    let raw = match field {
        Some(name) => fields.get(name)?,
        None => fields.get(VALUE_FIELD).or_else(|| {
            (fields.len() == 1)
                .then(|| fields.values().next())
                .flatten()
        })?,
    };
    let value = SecretString::new(raw.clone());
    if value.is_empty() {
        return None;
    }
    value.register_as(vw_secrets::SecretKind::VaultValue);
    Some(value)
}

/// The exact stored value for a user-initiated copy, including leading or trailing spaces in another app's item.
/// The result is registered for answer redaction, as [`value_of`] is.
#[must_use]
pub fn copy_value_of(item: &VaultItem) -> Option<SecretString> {
    agent_visible(item)?;
    let resource = VaultResource::from_item(item)?;
    let raw = resource.secret_fields.get(VALUE_FIELD).or_else(|| {
        (resource.secret_fields.len() == 1)
            .then(|| resource.secret_fields.values().next())
            .flatten()
    })?;
    if raw.is_empty() {
        return None;
    }
    let value = SecretString::exact(raw.clone());
    value.register_as(vw_secrets::SecretKind::VaultValue);
    Some(value)
}

/// The resource for a new or replaced secret (`id` from [`new_id`] or an existing own id).
#[must_use]
pub fn resource(
    id: &str,
    name: &str,
    value: &SecretString,
    source_origin: Option<&str>,
    label: Option<SecretLabel>,
    created_ms: Option<i64>,
    now_ms: i64,
) -> VaultResource {
    let mut public_fields = BTreeMap::new();
    public_fields.insert(
        "createdBy".to_owned(),
        Value::String("vibe-work".to_owned()),
    );
    if let Some(origin) = source_origin {
        public_fields.insert("sourceOrigin".to_owned(), Value::String(origin.to_owned()));
    }
    if let Some(label) = label {
        public_fields.insert(
            LABEL_FIELD.to_owned(),
            Value::String(label.as_str().to_owned()),
        );
    }
    let mut secret_fields = BTreeMap::new();
    secret_fields.insert(VALUE_FIELD.to_owned(), value.expose().to_owned());
    VaultResource {
        id: id.to_owned(),
        schema_version: crate::items::RESOURCE_SCHEMA_VERSION.to_owned(),
        kind: ResourceKind::DeveloperCredential,
        label: name.trim().to_owned(),
        permission: ResourcePermission::VisibleAsk,
        public_fields,
        secret_fields,
        custom_fields: Vec::new(),
        created_ms: created_ms.unwrap_or(now_ms),
        updated_ms: now_ms,
        extra: BTreeMap::new(),
    }
}
