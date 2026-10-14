//! Vault items and typed resources, stored the way ShellX Vault stores them, so every Vault app reads
//! what Vibe Work writes:
//! - an item is the JSON of [`VaultItem`] (schema of `vault-client/src/items.rs:37-56` @ the pin) at
//!   manifest path `.vault/items/<id>.json` (`items.rs:59`), chunked with `chunk_bytes`, each chunk
//!   sealed under `chunk_enc_key(chunk_enc_root, id)` with the chunk id hex as associated data;
//! - a typed resource is an item of type `resource` whose extra fields are the camelCase
//!   [`VaultResource`] (schema of `vault-broker/src/resources.rs:12-74`, conversion `:118-162`);
//! - every change commits a whole new sealed snapshot that keeps all other entries (incl. synced
//!   files) unchanged, parented on the head it was built on.
//!
//! Reading applies the three upstream gates (AEAD with the chunk id, plaintext re-hash equals the
//! keyed chunk id, length) and skips undecryptable items in listings like upstream `list_items`.
//!
//! Depends on: vault-core, `repo`, `chain` (through a guarded repository). Called by: `vault`, `mistral_key`.
//! Invariant: secrets in items live only in memory here; nothing is logged.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use vault_core::{
    ChunkRef, FileEntry, MasterKey, Snapshot, chunk_bytes, chunk_enc_key, chunk_id, crypto,
};

use crate::error::VaultError;
use crate::repo::{CommitOutcome, Head, LocalRepo};

/// Manifest prefix of vault items (`vault-client/src/engine.rs:41`).
pub const VAULT_PREFIX: &str = ".vault/";
/// Typed-resource schema version (`vault-broker/src/resources.rs:9`).
pub const RESOURCE_SCHEMA_VERSION: &str = "vault-resource-v1";
/// Commit attempts before reporting a conflict (upstream uses 3, `items.rs:64`).
pub const MAX_COMMIT_ATTEMPTS: u32 = 3;

/// Manifest path of an item.
#[must_use]
pub fn item_path(id: &str) -> String {
    format!("{VAULT_PREFIX}items/{id}.json")
}

/// One vault item, field for field the upstream JSON (unknown fields are kept).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultItem {
    /// Item id (the file name in the manifest).
    pub id: String,
    /// `login`, `note` or `resource`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Title shown in the apps.
    pub title: String,
    /// Login user name.
    #[serde(default)]
    pub username: String,
    /// Login password (or a ShellX compat secret value).
    #[serde(default)]
    pub password: String,
    /// Login URL.
    #[serde(default)]
    pub url: String,
    /// Notes (ShellX compat metadata lives here).
    #[serde(default)]
    pub notes: String,
    /// Creation time, ms.
    #[serde(default)]
    pub created_ms: i64,
    /// Last change, ms.
    #[serde(default)]
    pub updated_ms: i64,
    /// Fields of newer schemas (typed resources keep theirs here).
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for VaultItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultItem")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Typed resource kinds (`resources.rs:12-22`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    /// Website login.
    Login,
    /// Generic secret.
    Secret,
    /// Profile card.
    ProfileCard,
    /// Payment card.
    PaymentCard,
    /// Agent wallet.
    AgentWallet,
    /// Email inbox.
    EmailInbox,
    /// API key, token or other developer credential.
    DeveloperCredential,
}

/// Who may see a resource (`resources.rs:24-31`).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourcePermission {
    /// Hidden from every agent surface.
    UserOnly,
    /// Metadata visible to agents, use needs approval (upstream default).
    #[default]
    VisibleAsk,
    /// Usable through policy.
    AlwaysAllowed,
}

/// A custom field (`resources.rs:58-74`).
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomField {
    /// Field id.
    pub id: String,
    /// Label.
    pub label: String,
    /// Field kind.
    pub kind: String,
    /// Value (may be secret).
    pub value: String,
    /// Shown masked.
    #[serde(default)]
    pub concealed: bool,
    /// Autofill hint.
    #[serde(default)]
    pub autofill_hint: Option<String>,
    /// Sort order.
    #[serde(default)]
    pub order: i64,
    /// Newer fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A typed resource (`resources.rs:33-56`).
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VaultResource {
    /// Resource id (= item id).
    pub id: String,
    /// Schema version.
    #[serde(rename = "schemaVersion", default = "default_schema_version")]
    pub schema_version: String,
    /// Kind.
    pub kind: ResourceKind,
    /// Label (= item title).
    pub label: String,
    /// Agent visibility.
    #[serde(default)]
    pub permission: ResourcePermission,
    /// Non-secret fields.
    #[serde(default, rename = "publicFields")]
    pub public_fields: BTreeMap<String, Value>,
    /// Secret fields.
    #[serde(default, rename = "secretFields")]
    pub secret_fields: BTreeMap<String, String>,
    /// Custom fields.
    #[serde(default, rename = "customFields")]
    pub custom_fields: Vec<CustomField>,
    /// Creation time, ms.
    #[serde(default)]
    pub created_ms: i64,
    /// Last change, ms.
    #[serde(default)]
    pub updated_ms: i64,
    /// Newer fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for VaultResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultResource")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

fn default_schema_version() -> String {
    RESOURCE_SCHEMA_VERSION.to_owned()
}

impl VaultResource {
    /// The item that stores this resource (upstream `VaultResource::to_vault_item`).
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] if the resource does not serialize to an object (cannot happen for
    /// this type; kept typed instead of a panic).
    pub fn to_item(&self) -> Result<VaultItem, VaultError> {
        let Value::Object(map) =
            serde_json::to_value(self).map_err(|e| VaultError::Corrupt(e.to_string()))?
        else {
            return Err(VaultError::Corrupt("resource is not an object".to_owned()));
        };
        let mut extra: BTreeMap<String, Value> = map.into_iter().collect();
        extra.remove("id");
        Ok(VaultItem {
            id: self.id.clone(),
            kind: "resource".to_owned(),
            title: self.label.clone(),
            username: String::new(),
            password: String::new(),
            url: String::new(),
            notes: String::new(),
            created_ms: self.created_ms,
            updated_ms: self.updated_ms,
            extra,
        })
    }

    /// The resource stored in a `resource` item (upstream `from_typed_vault_item`), else `None`.
    #[must_use]
    pub fn from_item(item: &VaultItem) -> Option<Self> {
        if item.kind != "resource" {
            return None;
        }
        let mut object: Map<String, Value> = item.extra.clone().into_iter().collect();
        object
            .entry("id")
            .or_insert_with(|| Value::String(item.id.clone()));
        object
            .entry("label")
            .or_insert_with(|| Value::String(item.title.clone()));
        object
            .entry("createdMs")
            .or_insert_with(|| Value::Number(item.created_ms.into()));
        object
            .entry("updatedMs")
            .or_insert_with(|| Value::Number(item.updated_ms.into()));
        serde_json::from_value(Value::Object(object)).ok()
    }
}

/// The head snapshot, opened with the master key (`None` = empty repository). A guarded repository (every vault
/// session's) accepts only a head that follows the newest one this process has seen (`chain`).
///
/// # Errors
/// [`VaultError::Corrupt`] when the head does not open with this key or the guard refuses it, [`VaultError::Io`].
pub fn head_snapshot(
    repo: &LocalRepo,
    master: &MasterKey,
) -> Result<Option<(Head, Snapshot)>, VaultError> {
    if let Some(chain) = repo.chain() {
        return chain.head_snapshot(repo, master);
    }
    let Some(head) = repo.head()? else {
        return Ok(None);
    };
    let snapshot = Snapshot::open(master, &head.sealed).map_err(|_| {
        VaultError::Corrupt("the latest vault version does not open with this key".to_owned())
    })?;
    Ok(Some((head, snapshot)))
}

fn entry_bytes(
    repo: &LocalRepo,
    master: &MasterKey,
    entry: &FileEntry,
) -> Result<Vec<u8>, VaultError> {
    let enc_root = master.chunk_enc_root();
    let id_key = master.chunk_id_key();
    let mut out = Vec::new();
    for chunk in &entry.chunks {
        let hex_id = chunk.id.to_hex();
        let sealed = repo.blob(&hex_id)?;
        let plain = crypto::open(
            &chunk_enc_key(&enc_root, &chunk.id),
            hex_id.as_bytes(),
            &sealed,
        )
        .map_err(|_| VaultError::Corrupt("a chunk failed authentication".to_owned()))?;
        if chunk_id(&id_key, &plain) != chunk.id {
            return Err(VaultError::Corrupt(
                "a chunk does not match its id".to_owned(),
            ));
        }
        if u32::try_from(plain.len()).ok() != Some(chunk.size) {
            return Err(VaultError::Corrupt(
                "a chunk has the wrong length".to_owned(),
            ));
        }
        out.extend_from_slice(&plain);
    }
    Ok(out)
}

fn decode(bytes: &[u8]) -> Result<VaultItem, VaultError> {
    serde_json::from_slice(bytes)
        .map_err(|_| VaultError::Corrupt("an item is not valid JSON".to_owned()))
}

/// One item by id (`None` when absent).
///
/// # Errors
/// [`VaultError::Corrupt`], [`VaultError::Io`].
pub fn read_item(
    repo: &LocalRepo,
    master: &MasterKey,
    id: &str,
) -> Result<Option<VaultItem>, VaultError> {
    let Some((_, snapshot)) = head_snapshot(repo, master)? else {
        return Ok(None);
    };
    let Some(entry) = snapshot.file(&item_path(id)) else {
        return Ok(None);
    };
    decode(&entry_bytes(repo, master, entry)?).map(Some)
}

/// Every decryptable item (damaged ones are skipped, as in upstream `list_items`), by title.
///
/// # Errors
/// [`VaultError::Corrupt`] when the head itself does not open, [`VaultError::Io`].
pub fn list_items(repo: &LocalRepo, master: &MasterKey) -> Result<Vec<VaultItem>, VaultError> {
    let Some((_, snapshot)) = head_snapshot(repo, master)? else {
        return Ok(Vec::new());
    };
    let mut items: Vec<VaultItem> = snapshot
        .files
        .iter()
        .filter(|f| f.path.starts_with(VAULT_PREFIX))
        .filter_map(|entry| entry_bytes(repo, master, entry).ok())
        .filter_map(|bytes| decode(&bytes).ok())
        .collect();
    items.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(items)
}

fn valid_item_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '\\']) && id != "." && id != ".."
}

/// Creates or replaces an item. Returns the new generation.
///
/// # Errors
/// [`VaultError::Conflict`] after [`MAX_COMMIT_ATTEMPTS`] moved heads, [`VaultError::Corrupt`],
/// [`VaultError::Io`].
pub fn upsert_item(
    repo: &LocalRepo,
    master: &MasterKey,
    device: &str,
    item: &VaultItem,
) -> Result<u64, VaultError> {
    if !valid_item_id(&item.id) {
        return Err(VaultError::Corrupt("invalid item id".to_owned()));
    }
    let entry = seal_item(repo, master, item)?;
    mutate(repo, master, device, |files| {
        files.retain(|f| f.path != entry.path);
        files.push(entry.clone());
    })
}

/// Stores `item`'s bytes as sealed chunks and returns its file entry (not yet committed).
fn seal_item(
    repo: &LocalRepo,
    master: &MasterKey,
    item: &VaultItem,
) -> Result<FileEntry, VaultError> {
    let mut data = serde_json::to_vec(item).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    let enc_root = master.chunk_enc_root();
    let id_key = master.chunk_id_key();
    let mut refs = Vec::new();
    for chunk in chunk_bytes(&data) {
        let id = chunk_id(&id_key, &chunk.data);
        let sealed = crypto::seal(
            &chunk_enc_key(&enc_root, &id),
            id.to_hex().as_bytes(),
            &chunk.data,
        );
        repo.put_blob(&id.to_hex(), &sealed)?;
        refs.push(ChunkRef {
            id,
            size: u32::try_from(chunk.data.len())
                .map_err(|_| VaultError::Corrupt("item too large".to_owned()))?,
        });
    }
    let entry = FileEntry {
        path: item_path(&item.id),
        executable: false,
        mtime_ns: item.updated_ms.saturating_mul(1_000_000),
        size: data.len() as u64,
        chunks: refs,
    };
    zeroize::Zeroize::zeroize(&mut data);
    Ok(entry)
}

/// an atomic read-modify-write of item `id`. Under the repository lock (every Vibe Work writer takes it)
/// and on the head it read (another app moving the head makes it read again), `change` gets the item as it is in
/// that head and returns the item to write, or `None` to write nothing. Returns the item as it is after the call
/// (`None`: absent). A change computed from an older head is never committed, so a concurrent writer's change to
/// the same item is never lost.
///
/// # Errors
/// As [`upsert_item`].
pub fn update_item(
    repo: &LocalRepo,
    master: &MasterKey,
    device: &str,
    id: &str,
    change: impl Fn(Option<VaultItem>) -> Option<VaultItem>,
) -> Result<Option<VaultItem>, VaultError> {
    let path = item_path(id);
    let lock = repo.lock()?;
    for _ in 0..MAX_COMMIT_ATTEMPTS {
        let head = head_snapshot(repo, master)?;
        let (parent, mut files) = match &head {
            Some((head, snapshot)) => (Some(head.id), snapshot.files.clone()),
            None => (None, Vec::new()),
        };
        let current = match files.iter().find(|f| f.path == path) {
            Some(entry) => Some(decode(&entry_bytes(repo, master, entry)?)?),
            None => None,
        };
        let Some(next) = change(current.clone()) else {
            return Ok(current);
        };
        if next.id != id || !valid_item_id(id) {
            return Err(VaultError::Corrupt(
                "an update may not change the item id".to_owned(),
            ));
        }
        let entry = seal_item(repo, master, &next)?;
        files.retain(|f| f.path != path);
        files.push(entry);
        let sealed = Snapshot::new(parent, device, now_ms(), files).seal(master);
        match repo.commit_if_head_locked(&lock, head.as_ref().map(|(h, _)| h), &sealed)? {
            CommitOutcome::Committed(_) => return Ok(Some(next)),
            CommitOutcome::HeadMoved => {}
        }
    }
    Err(VaultError::Conflict {
        attempts: MAX_COMMIT_ATTEMPTS,
    })
}

/// Removes an item (absent is fine). Returns the new generation, or the current one when nothing
/// had to change.
///
/// # Errors
/// As [`upsert_item`].
pub fn delete_item(
    repo: &LocalRepo,
    master: &MasterKey,
    device: &str,
    id: &str,
) -> Result<u64, VaultError> {
    let path = item_path(id);
    mutate(repo, master, device, |files| {
        files.retain(|f| f.path != path);
    })
}

/// Deletes one sealed item only if its current decoded value passes `accept` in the same head transaction.
/// `None` means absent; `Some(false)` means rejected; `Some(true)` means deleted. A moved head retries the
/// predicate against the new value, so a corrupt replacement can never be removed by a stale validation.
///
/// # Errors
/// As [`upsert_item`].
pub fn delete_item_when(
    repo: &LocalRepo,
    master: &MasterKey,
    device: &str,
    id: &str,
    accept: impl Fn(&VaultItem) -> bool,
) -> Result<Option<bool>, VaultError> {
    let path = item_path(id);
    let lock = repo.lock()?;
    for _ in 0..MAX_COMMIT_ATTEMPTS {
        let head = head_snapshot(repo, master)?;
        let (parent, mut files) = match &head {
            Some((head, snapshot)) => (Some(head.id), snapshot.files.clone()),
            None => (None, Vec::new()),
        };
        let Some(entry) = files.iter().find(|f| f.path == path) else {
            return Ok(None);
        };
        let current = decode(&entry_bytes(repo, master, entry)?)?;
        if !accept(&current) {
            return Ok(Some(false));
        }
        files.retain(|f| f.path != path);
        let sealed = Snapshot::new(parent, device, now_ms(), files).seal(master);
        match repo.commit_if_head_locked(&lock, head.as_ref().map(|(h, _)| h), &sealed)? {
            CommitOutcome::Committed(_) => return Ok(Some(true)),
            CommitOutcome::HeadMoved => {}
        }
    }
    Err(VaultError::Conflict {
        attempts: MAX_COMMIT_ATTEMPTS,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Builds the next snapshot on the current head and commits it, holding the repository lock for the
/// whole cycle (Vibe Work writers never race each other). A head moved by another app (ShellX, the
/// Vault app) makes it start again from the new head (the change is re-applied, never merged
/// blindly), at most [`MAX_COMMIT_ATTEMPTS`] times, then it fails loudly with
/// [`VaultError::Conflict`].
fn mutate(
    repo: &LocalRepo,
    master: &MasterKey,
    device: &str,
    change: impl Fn(&mut Vec<FileEntry>),
) -> Result<u64, VaultError> {
    let lock = repo.lock()?;
    for _ in 0..MAX_COMMIT_ATTEMPTS {
        let head = head_snapshot(repo, master)?;
        let (parent, mut files) = match &head {
            Some((head, snapshot)) => (Some(head.id), snapshot.files.clone()),
            None => (None, Vec::new()),
        };
        let before = files.clone();
        change(&mut files);
        if let (Some((head, _)), true) = (&head, files == before) {
            return Ok(head.generation);
        }
        let sealed = Snapshot::new(parent, device, now_ms(), files).seal(master);
        match repo.commit_if_head_locked(&lock, head.as_ref().map(|(h, _)| h), &sealed)? {
            CommitOutcome::Committed(generation) => return Ok(generation),
            CommitOutcome::HeadMoved => {}
        }
    }
    Err(VaultError::Conflict {
        attempts: MAX_COMMIT_ATTEMPTS,
    })
}
