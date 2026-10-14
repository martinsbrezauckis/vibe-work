//! The secret broker's command grants (mirroring `mcp_binding`): "always for this command" answers
//! of the `secret_run` dialog, kept in the vault instead of a file of Vibe Work's. An agent runs as the same user and
//! could write a plain file, planting a grant for a command of its own that then gets the secret with no dialog; a
//! vault resource needs the vault's unlock material.
//!
//! Format: one typed resource per (project, pinned command), kind `secret`, permission `userOnly` (never listed to an
//! agent, never a project secret), id `vibe-work-grant-<16 hex>` derived from the project and the fingerprint
//! ([`id_for`]: a use is one read, not a scan), label `Command grant <program> (Vibe Work)`, no secret fields,
//! public fields: `createdBy = "vibe-work"`, `project`, `fingerprint`, `command`, `bindings`, `secretIds`, `always`,
//! `grantedMs`, `revokedMs`. A revoke keeps the record (`always` off, `revokedMs` set) so a dialog that was open
//! during the revoke can see it; a later "always" for the same command turns the same record on again.
//!
//! Depends on: `items`, blake3. Called by: vw-keys (`grants`).
//! Invariants: [`CommandGrant`] never holds a value; only resources with the derived id shape, `userOnly` and
//! `createdBy = "vibe-work"` and every field of the right type parse as grants (anything else is ignored, never
//! guessed); a grant permits only while [`CommandGrant::active`].

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::items::{ResourceKind, ResourcePermission, VaultItem, VaultResource};

/// Id prefix of grant resources.
pub const ID_PREFIX: &str = "vibe-work-grant-";

/// One "always for this command" grant (metadata only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandGrant {
    /// Resource id ([`id_for`] of `project` and `fingerprint`).
    pub id: String,
    /// The project folder the grant covers (its sessions).
    pub project: String,
    /// The pinned command's fingerprint (the secret broker's `secret_pin`).
    pub fingerprint: String,
    /// The command as the dialog showed it (display only).
    pub command: String,
    /// `VARIABLE <- secret name` per secret (display only).
    pub bindings: Vec<String>,
    /// The secrets' ids (removing a secret revokes its grants).
    pub secret_ids: Vec<String>,
    /// The user's "always" answer is in force (false after a revoke).
    pub always: bool,
    /// When the user last chose "always", ms.
    pub granted_ms: i64,
    /// When the grant was last revoked, ms (0: never).
    pub revoked_ms: i64,
    /// Creation time of the record, ms.
    pub created_ms: i64,
}

impl CommandGrant {
    /// True while the grant permits a run: "always" was chosen after the last revoke.
    #[must_use]
    pub fn active(&self) -> bool {
        self.always && self.granted_ms > self.revoked_ms
    }
}

/// The project folder as part of an id: without case on Windows and macOS (their folders compare without case).
fn project_key(project: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        project.to_lowercase()
    } else {
        project.to_owned()
    }
}

/// The record id of the grant of `fingerprint` in `project` (the first 8 bytes of a domain-separated BLAKE3, hex).
#[must_use]
pub fn id_for(project: &str, fingerprint: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vibe-work-grant-v1\0");
    hasher.update(project_key(project).as_bytes());
    hasher.update(b"\0");
    hasher.update(fingerprint.as_bytes());
    format!(
        "{ID_PREFIX}{}",
        hex::encode(&hasher.finalize().as_bytes()[..8])
    )
}

/// True when `id` has the shape of a grant id.
#[must_use]
pub fn is_grant_id(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX).is_some_and(|rest| {
        rest.len() == 16
            && rest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

/// True when the two project folders are the same project (as [`id_for`] compares them).
#[must_use]
pub fn same_project(a: &str, b: &str) -> bool {
    project_key(a) == project_key(b)
}

fn text(fields: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key)?.as_str().map(str::to_owned)
}

fn texts(fields: &BTreeMap<String, Value>, key: &str) -> Option<Vec<String>> {
    fields
        .get(key)?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect()
}

/// The grant stored in `item`, if it is one (module docs). The id must also be the one [`id_for`] gives for the
/// record's own project and fingerprint.
#[must_use]
pub fn from_item(item: &VaultItem) -> Option<CommandGrant> {
    let resource = VaultResource::from_item(item)?;
    if !is_grant_id(&resource.id)
        || resource.kind != ResourceKind::Secret
        || resource.permission != ResourcePermission::UserOnly
        || !resource.secret_fields.is_empty()
        || text(&resource.public_fields, "createdBy").as_deref() != Some("vibe-work")
    {
        return None;
    }
    let f = &resource.public_fields;
    let grant = CommandGrant {
        project: text(f, "project")?,
        fingerprint: text(f, "fingerprint")?,
        command: text(f, "command")?,
        bindings: texts(f, "bindings")?,
        secret_ids: texts(f, "secretIds")?,
        always: f.get("always")?.as_bool()?,
        granted_ms: f.get("grantedMs")?.as_i64()?,
        revoked_ms: f.get("revokedMs")?.as_i64()?,
        created_ms: resource.created_ms,
        id: resource.id,
    };
    (grant.id == id_for(&grant.project, &grant.fingerprint)).then_some(grant)
}

/// The resource that stores `grant` (written at `now_ms`).
#[must_use]
pub fn resource(grant: &CommandGrant, now_ms: i64) -> VaultResource {
    let strings = |list: &[String]| Value::Array(list.iter().cloned().map(Value::String).collect());
    let mut f = BTreeMap::new();
    f.insert(
        "createdBy".to_owned(),
        Value::String("vibe-work".to_owned()),
    );
    f.insert("project".to_owned(), Value::String(grant.project.clone()));
    f.insert(
        "fingerprint".to_owned(),
        Value::String(grant.fingerprint.clone()),
    );
    f.insert("command".to_owned(), Value::String(grant.command.clone()));
    f.insert("bindings".to_owned(), strings(&grant.bindings));
    f.insert("secretIds".to_owned(), strings(&grant.secret_ids));
    f.insert("always".to_owned(), Value::Bool(grant.always));
    f.insert("grantedMs".to_owned(), Value::from(grant.granted_ms));
    f.insert("revokedMs".to_owned(), Value::from(grant.revoked_ms));
    let program = grant
        .command
        .split_whitespace()
        .next()
        .and_then(|p| p.rsplit(['/', '\\']).next())
        .unwrap_or("command");
    VaultResource {
        id: grant.id.clone(),
        schema_version: crate::items::RESOURCE_SCHEMA_VERSION.to_owned(),
        kind: ResourceKind::Secret,
        label: format!("Command grant {program} (Vibe Work)"),
        permission: ResourcePermission::UserOnly,
        public_fields: f,
        secret_fields: BTreeMap::new(),
        custom_fields: Vec::new(),
        created_ms: grant.created_ms,
        updated_ms: now_ms,
        extra: BTreeMap::new(),
    }
}
