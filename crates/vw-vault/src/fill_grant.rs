//! Fill grants (the shape of `command_grant`): the user's "Always for this site" answer to the
//! native dialog that lets the agent fill a vault login's password on one site. Kept in the vault, not in a file of
//! Vibe Work's: an agent runs as the same user and could plant a plain file; a vault record needs the vault's unlock
//! material, and the head guard (`chain`) refuses an older vault version copied back.
//!
//! Format: one typed resource per (login, exact origin), kind `secret`, permission `userOnly` (never listed to an
//! agent), id `vibe-work-fill-<16 hex>` derived from the login id and the origin ([`id_for`]: a use is one read),
//! label `Fill grant <origin> (Vibe Work)`, no secret fields, public fields `createdBy = "vibe-work"`, `loginId`,
//! `loginName`, `origin`, `always`, `grantedMs`, `revokedMs`. A revoke keeps the record (`always` off, `revokedMs`
//! set) so a dialog that was open during the revoke can see it.
//!
//! Depends on: `items`, blake3. Called by: vw-keys (`logins`).
//! Invariants: [`FillGrant`] never holds a value; only resources with the derived id, `userOnly`, `createdBy =
//! "vibe-work"` and every field of the right type parse as grants; a grant permits only while [`FillGrant::active`].

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::items::{ResourceKind, ResourcePermission, VaultItem, VaultResource};

/// Id prefix of fill grant resources.
pub const ID_PREFIX: &str = "vibe-work-fill-";

/// One "always for this site" grant (metadata only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FillGrant {
    /// Resource id ([`id_for`] of `login_id` and `origin`).
    pub id: String,
    /// The login's item id.
    pub login_id: String,
    /// The login's name when the user allowed it (display only).
    pub login_name: String,
    /// The exact page origin (`https://accounts.example.com`).
    pub origin: String,
    /// The user's "always" answer is in force (false after a revoke).
    pub always: bool,
    /// When the user last chose "always", ms.
    pub granted_ms: i64,
    /// When the grant was last revoked, ms (0: never).
    pub revoked_ms: i64,
    /// Creation time of the record, ms.
    pub created_ms: i64,
}

impl FillGrant {
    /// True while the grant permits a fill: "always" was chosen after the last revoke.
    #[must_use]
    pub fn active(&self) -> bool {
        self.always && self.granted_ms > self.revoked_ms
    }
}

/// The record id of the grant of login `login_id` on `origin` (the first 8 bytes of a domain-separated BLAKE3, hex).
#[must_use]
pub fn id_for(login_id: &str, origin: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vibe-work-fill-grant-v1\0");
    hasher.update(login_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(origin.to_ascii_lowercase().as_bytes());
    format!(
        "{ID_PREFIX}{}",
        hex::encode(&hasher.finalize().as_bytes()[..8])
    )
}

/// True when `id` has the shape of a fill grant id.
#[must_use]
pub fn is_grant_id(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX).is_some_and(|rest| {
        rest.len() == 16
            && rest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

fn text(fields: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key)?.as_str().map(str::to_owned)
}

/// The grant stored in `item`, if it is one (module docs). The id must be the one [`id_for`] gives for the record's
/// own login and origin.
#[must_use]
pub fn from_item(item: &VaultItem) -> Option<FillGrant> {
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
    let grant = FillGrant {
        login_id: text(f, "loginId")?,
        login_name: text(f, "loginName")?,
        origin: text(f, "origin")?,
        always: f.get("always")?.as_bool()?,
        granted_ms: f.get("grantedMs")?.as_i64()?,
        revoked_ms: f.get("revokedMs")?.as_i64()?,
        created_ms: resource.created_ms,
        id: resource.id,
    };
    (grant.id == id_for(&grant.login_id, &grant.origin)).then_some(grant)
}

/// The resource that stores `grant` (written at `now_ms`).
#[must_use]
pub fn resource(grant: &FillGrant, now_ms: i64) -> VaultResource {
    let mut f = BTreeMap::new();
    f.insert(
        "createdBy".to_owned(),
        Value::String("vibe-work".to_owned()),
    );
    f.insert("loginId".to_owned(), Value::String(grant.login_id.clone()));
    f.insert(
        "loginName".to_owned(),
        Value::String(grant.login_name.clone()),
    );
    f.insert("origin".to_owned(), Value::String(grant.origin.clone()));
    f.insert("always".to_owned(), Value::Bool(grant.always));
    f.insert("grantedMs".to_owned(), Value::from(grant.granted_ms));
    f.insert("revokedMs".to_owned(), Value::from(grant.revoked_ms));
    VaultResource {
        id: grant.id.clone(),
        schema_version: crate::items::RESOURCE_SCHEMA_VERSION.to_owned(),
        kind: ResourceKind::Secret,
        label: format!("Fill grant {} (Vibe Work)", grant.origin),
        permission: ResourcePermission::UserOnly,
        public_fields: f,
        secret_fields: BTreeMap::new(),
        custom_fields: Vec::new(),
        created_ms: grant.created_ms,
        updated_ms: now_ms,
        extra: BTreeMap::new(),
    }
}
