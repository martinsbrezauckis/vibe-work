//! Project-scoped agent permission for one Vault entry. Vibe Work owns this hidden metadata record, including when
//! the entry belongs to another Vault app; it never changes that entry or stores its value.
//!
//! Depends on: `items`, blake3. Called by: `vw-keys` and the core's agent-use checks.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::items::{ResourceKind, ResourcePermission, VaultItem, VaultResource};

/// Id prefix of a Vibe Work project entry policy.
pub const ID_PREFIX: &str = "vibe-work-entry-permission-";
/// Distinct hidden record id for an entry-wide default.
pub const DEFAULT_ID_PREFIX: &str = "vibe-work-entry-default-";

/// The user's choice for this entry in this project. An absent record means Ask each time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryPermission {
    /// Ask under the existing operation-specific grant flow.
    #[default]
    AskEachTime,
    /// Standing permission for this entry and project.
    Allowed,
    /// Refuse all agent uses of this entry in this project.
    Never,
}

impl EntryPermission {
    /// Stored and wire word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AskEachTime => "askEachTime",
            Self::Allowed => "allowed",
            Self::Never => "never",
        }
    }
}

/// Match the project spelling used by command grants across case-insensitive hosts.
fn project_key(project: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        project.to_lowercase()
    } else {
        project.to_owned()
    }
}

/// Derives the record id from the project and entry id.
#[must_use]
pub fn id_for(project: &str, entry_id: &str) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"vibe-work-entry-permission-v1\0");
    hash.update(project_key(project).as_bytes());
    hash.update(b"\0");
    hash.update(entry_id.as_bytes());
    format!(
        "{ID_PREFIX}{}",
        hex::encode(&hash.finalize().as_bytes()[..8])
    )
}

/// Derives the entry-wide default record id.
#[must_use]
pub fn default_id_for(entry_id: &str) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"vibe-work-entry-default-v1\0");
    hash.update(entry_id.as_bytes());
    format!(
        "{DEFAULT_ID_PREFIX}{}",
        hex::encode(&hash.finalize().as_bytes()[..8])
    )
}

/// Sealed, user-only default permission for a secret or login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultPolicy {
    /// Derived record id.
    pub id: String,
    /// Referenced secret or login id.
    pub entry_id: String,
    /// User choice.
    pub permission: EntryPermission,
    /// Last user change.
    pub changed_ms: i64,
    /// Record creation time.
    pub created_ms: i64,
}

/// Strictly parses a sealed default record.
#[must_use]
pub fn default_from_item(item: &VaultItem) -> Option<DefaultPolicy> {
    let resource = VaultResource::from_item(item)?;
    let fields = &resource.public_fields;
    if resource.kind != ResourceKind::Secret
        || resource.permission != ResourcePermission::UserOnly
        || !resource.secret_fields.is_empty()
        || fields.get("createdBy")?.as_str()? != "vibe-work"
    {
        return None;
    }
    let entry_id = fields.get("entryId")?.as_str()?.to_owned();
    let policy = DefaultPolicy {
        id: resource.id,
        entry_id,
        permission: serde_json::from_value(fields.get("permission")?.clone()).ok()?,
        changed_ms: fields.get("changedMs")?.as_i64()?,
        created_ms: resource.created_ms,
    };
    (policy.id == default_id_for(&policy.entry_id)).then_some(policy)
}

/// Builds the hidden user-only resource for an entry-wide default.
#[must_use]
pub fn default_resource(policy: &DefaultPolicy) -> VaultResource {
    let mut fields = BTreeMap::new();
    for (key, value) in [
        ("createdBy", Value::String("vibe-work".to_owned())),
        ("entryId", Value::String(policy.entry_id.clone())),
        (
            "permission",
            Value::String(policy.permission.as_str().to_owned()),
        ),
        ("changedMs", Value::from(policy.changed_ms)),
    ] {
        fields.insert(key.to_owned(), value);
    }
    VaultResource {
        id: policy.id.clone(),
        schema_version: crate::items::RESOURCE_SCHEMA_VERSION.to_owned(),
        kind: ResourceKind::Secret,
        label: "Vibe Work entry default permission".to_owned(),
        permission: ResourcePermission::UserOnly,
        public_fields: fields,
        secret_fields: BTreeMap::new(),
        custom_fields: Vec::new(),
        created_ms: policy.created_ms,
        updated_ms: policy.changed_ms,
        extra: BTreeMap::new(),
    }
}

/// Metadata record sealed by the Vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPolicy {
    /// Derived record id.
    pub id: String,
    /// Project folder.
    pub project: String,
    /// Referenced secret or login id.
    pub entry_id: String,
    /// User choice.
    pub permission: EntryPermission,
    /// Last user change, including revocation.
    pub changed_ms: i64,
    /// Record creation time.
    pub created_ms: i64,
}

/// Strictly parses a sealed policy record.
#[must_use]
pub fn from_item(item: &VaultItem) -> Option<EntryPolicy> {
    let resource = VaultResource::from_item(item)?;
    let fields = &resource.public_fields;
    if resource.kind != ResourceKind::Secret
        || resource.permission != ResourcePermission::UserOnly
        || !resource.secret_fields.is_empty()
        || fields.get("createdBy")?.as_str()? != "vibe-work"
    {
        return None;
    }
    let project = fields.get("project")?.as_str()?.to_owned();
    let entry_id = fields.get("entryId")?.as_str()?.to_owned();
    let permission = serde_json::from_value(fields.get("permission")?.clone()).ok()?;
    let policy = EntryPolicy {
        id: resource.id,
        project,
        entry_id,
        permission,
        changed_ms: fields.get("changedMs")?.as_i64()?,
        created_ms: resource.created_ms,
    };
    (policy.id == id_for(&policy.project, &policy.entry_id)).then_some(policy)
}

/// Builds the hidden user-only resource for a policy.
#[must_use]
pub fn resource(policy: &EntryPolicy) -> VaultResource {
    let mut fields = BTreeMap::new();
    for (key, value) in [
        ("createdBy", Value::String("vibe-work".to_owned())),
        ("project", Value::String(policy.project.clone())),
        ("entryId", Value::String(policy.entry_id.clone())),
        (
            "permission",
            Value::String(policy.permission.as_str().to_owned()),
        ),
        ("changedMs", Value::from(policy.changed_ms)),
    ] {
        fields.insert(key.to_owned(), value);
    }
    VaultResource {
        id: policy.id.clone(),
        schema_version: crate::items::RESOURCE_SCHEMA_VERSION.to_owned(),
        kind: ResourceKind::Secret,
        label: "Vibe Work project entry permission".to_owned(),
        permission: ResourcePermission::UserOnly,
        public_fields: fields,
        secret_fields: BTreeMap::new(),
        custom_fields: Vec::new(),
        created_ms: policy.created_ms,
        updated_ms: policy.changed_ms,
        extra: BTreeMap::new(),
    }
}
