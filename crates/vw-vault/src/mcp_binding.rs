//! MCP server key bindings: which vault secret Vibe Work's MCP forwarder puts into which header for which
//! MCP server, and whether the user allowed it "always". Kept in the vault rather than in a file of Vibe Work's: an
//! agent runs as the same user and could write a plain file, planting a binding that sends a key to its own server at
//! the next start; a vault resource needs the vault's unlock material.
//!
//! Format: one typed resource per server, kind `secret`, permission `userOnly` (never listed to an agent, never a
//! project secret), id `vibe-work-mcp-<16 hex>`, label `MCP server <name> (Vibe Work)`, no secret fields (the key
//! stays in its own secret, referenced by id), public fields: `createdBy = "vibe-work"`, `mcpServer`, `upstream`,
//! `header`, `bearer`, `secretId`, `secretName`, `always`, `revokedMs`.
//!
//! Depends on: `items`. Called by: vw-keys (`mcp`).
//! Invariants: [`McpBinding`] never holds a value; only resources with [`ID_PREFIX`] and `createdBy = "vibe-work"`
//! parse as bindings; a resource with a missing or mistyped field is not a binding (ignored, never guessed).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::items::{ResourceKind, ResourcePermission, VaultItem, VaultResource};

/// Id prefix of binding resources.
pub const ID_PREFIX: &str = "vibe-work-mcp-";

/// One MCP server's key binding (metadata only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBinding {
    /// Resource id (also the forwarder path segment).
    pub id: String,
    /// The server's name in Vibe's config.
    pub server: String,
    /// The address the key goes to (canonical, checked when added).
    pub upstream: String,
    /// The header that carries the key.
    pub header: String,
    /// True: the header value is `Bearer <key>`; false: the key as is.
    pub bearer: bool,
    /// The secret's resource id.
    pub secret_id: String,
    /// The secret's name when the binding was made (shown; the id is what counts).
    pub secret_name: String,
    /// The user's "always" grant.
    pub always: bool,
    /// When the user last revoked it, ms (0: never). Grants "until Vibe Work closes" given before it are void, in
    /// every process that shares the vault.
    pub revoked_ms: i64,
    /// Creation time, ms.
    pub created_ms: i64,
}

/// True when `id` has the shape of a binding id.
#[must_use]
pub fn is_binding_id(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX).is_some_and(|rest| {
        rest.len() == 16
            && rest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

/// A new random binding id.
#[must_use]
pub fn new_id() -> String {
    format!(
        "{ID_PREFIX}{}",
        hex::encode(vault_core::random_bytes::<8>())
    )
}

fn text(fields: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key)?.as_str().map(str::to_owned)
}

/// The binding stored in `item`, if it is one (module docs).
#[must_use]
pub fn from_item(item: &VaultItem) -> Option<McpBinding> {
    let resource = VaultResource::from_item(item)?;
    if !is_binding_id(&resource.id)
        || resource.permission != ResourcePermission::UserOnly
        || text(&resource.public_fields, "createdBy").as_deref() != Some("vibe-work")
    {
        return None;
    }
    let f = &resource.public_fields;
    Some(McpBinding {
        server: text(f, "mcpServer")?,
        upstream: text(f, "upstream")?,
        header: text(f, "header")?,
        bearer: f.get("bearer")?.as_bool()?,
        secret_id: text(f, "secretId")?,
        secret_name: text(f, "secretName")?,
        always: f.get("always")?.as_bool()?,
        revoked_ms: f.get("revokedMs").and_then(Value::as_i64).unwrap_or(0),
        created_ms: resource.created_ms,
        id: resource.id,
    })
}

/// The resource that stores `binding` (written at `now_ms`).
#[must_use]
pub fn resource(binding: &McpBinding, now_ms: i64) -> VaultResource {
    let mut f = BTreeMap::new();
    f.insert(
        "createdBy".to_owned(),
        Value::String("vibe-work".to_owned()),
    );
    f.insert(
        "mcpServer".to_owned(),
        Value::String(binding.server.clone()),
    );
    f.insert(
        "upstream".to_owned(),
        Value::String(binding.upstream.clone()),
    );
    f.insert("header".to_owned(), Value::String(binding.header.clone()));
    f.insert("bearer".to_owned(), Value::Bool(binding.bearer));
    f.insert(
        "secretId".to_owned(),
        Value::String(binding.secret_id.clone()),
    );
    f.insert(
        "secretName".to_owned(),
        Value::String(binding.secret_name.clone()),
    );
    f.insert("always".to_owned(), Value::Bool(binding.always));
    f.insert("revokedMs".to_owned(), Value::from(binding.revoked_ms));
    VaultResource {
        id: binding.id.clone(),
        schema_version: crate::items::RESOURCE_SCHEMA_VERSION.to_owned(),
        kind: ResourceKind::Secret,
        label: format!("MCP server {} (Vibe Work)", binding.server),
        permission: ResourcePermission::UserOnly,
        public_fields: f,
        secret_fields: BTreeMap::new(),
        custom_fields: Vec::new(),
        created_ms: binding.created_ms,
        updated_ms: now_ms,
        extra: BTreeMap::new(),
    }
}
