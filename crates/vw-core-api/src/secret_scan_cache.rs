//! Incremental log stamps and counts in app data; no values, individual value hashes or redaction ranges.
//! Depends on serde, blake3 and filesystem metadata; called only by the scanner's background worker.

use crate::{SecretLeakFinding, SecretLeakName};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use vw_keys::VaultScanSecret;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stamp {
    pub size: u64,
    pub modified_ns: u128,
}

impl Stamp {
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        Ok(Self {
            size: meta.len(),
            modified_ns: meta
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedFile {
    pub stamp: Stamp,
    pub fingerprint: String,
    pub chat_id: String,
    pub title: String,
    pub hits: BTreeMap<String, SecretLeakName>,
    pub places: u32,
}

const POLICY_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub(crate) struct Cache {
    #[serde(default)]
    policy_version: u32,
    pub files: BTreeMap<PathBuf, CachedFile>,
    pub findings: Vec<SecretLeakFinding>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            policy_version: POLICY_VERSION,
            files: BTreeMap::new(),
            findings: Vec::new(),
        }
    }
}

impl Cache {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            // Findings produced before strength admission cannot be published or grant authority.
            .filter(|cache: &Self| cache.policy_version == POLICY_VERSION)
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            vw_platform::ensure_private_dir(parent).map_err(std::io::Error::other)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(temp, path)
    }
    pub fn changed(&self, path: &Path, stamp: &Stamp, fingerprint: &str) -> bool {
        self.files
            .get(path)
            .is_none_or(|old| old.stamp != *stamp || old.fingerprint != fingerprint)
    }
}

pub(crate) fn fingerprint(secrets: &[VaultScanSecret]) -> String {
    let mut ids: Vec<_> = secrets
        .iter()
        .map(|secret| (&secret.id, secret.modified_ms))
        .collect();
    ids.sort_unstable();
    let mut hash = blake3::Hasher::new();
    for (id, modified) in ids {
        hash.update(&(id.len() as u64).to_le_bytes());
        hash.update(id.as_bytes());
        hash.update(&modified.to_le_bytes());
    }
    hash.finalize().to_hex().to_string()
}

pub(crate) fn clean(text: &str) -> String {
    let bytes = vw_secrets::scrub(text.as_bytes())
        .map_or_else(|| text.as_bytes().to_vec(), |(bytes, _)| bytes);
    String::from_utf8_lossy(&bytes)
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}

/// `text` with every run of at least six characters that also occurs in a vault value withheld: a chat's
/// auto-title is made from its first message, so it can carry part of a value the whole-value scrub misses.
pub(crate) fn withhold_parts(text: &str, values: &[&str]) -> String {
    const MIN: usize = 6;
    let text: Vec<char> = text.chars().collect();
    let mut hidden = vec![false; text.len()];
    for value in values {
        let value: Vec<char> = value.chars().collect();
        for i in 0..text.len() {
            for j in 0..value.len() {
                let n = text[i..]
                    .iter()
                    .zip(&value[j..])
                    .take_while(|(a, b)| a == b)
                    .count();
                if n >= MIN {
                    hidden[i..i + n].fill(true);
                }
            }
        }
    }
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        if hidden[i] {
            out.push_str("[secret withheld]");
            while i < text.len() && hidden[i] {
                i += 1;
            }
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    out
}

/// Read only ownership fields with serde; other metadata and legacy message arrays are skipped.
#[derive(Deserialize)]
struct LogOwner {
    session_id: Option<String>,
    title: Option<String>,
    metadata: Option<Box<LogOwner>>,
}

/// The Vibe 3.0 store folder that holds `path`, `<folder>/unified/<id>/`, where `folder` is the scanned session log
/// folder (Vibe's `save_dir`). This is the layout of the pinned Vibe 3.0.x: `meta.json`, `CURRENT`,
/// `journal/*.jsonl`, `generations/<n>/*.json` and `chunks/<sha256>.json` below it. `None` for anything else.
pub(crate) fn unified_store(folder: &Path, path: &Path) -> Option<PathBuf> {
    let mut parts = path.strip_prefix(folder).ok()?.components();
    if parts.next()?.as_os_str() != "unified" {
        return None;
    }
    let id = parts.next()?;
    // A file directly in `unified/` is no store's file.
    parts.next()?;
    Some(folder.join("unified").join(id.as_os_str()))
}

/// True when `path` (in the scanned `folder`) must never be edited in place: it belongs to a Vibe 3.0 unified
/// store. That store is sealed by digests (each chunk is named by the sha256 of its body; the checkpoint, runtime
/// and projection documents are named with their sha256 in the generation's manifest, the manifest's digest is in
/// `CURRENT`, journal records are chained by digest, and documents embed digests of their own content), and Vibe
/// 3.0.2 checks them when it loads the chat (`mistralai_vibe_local_harness/vibe/_storage.py`: "stored chunk digest
/// mismatch", "stored record digest mismatch"). A replaced value would make the chat unloadable, so the scanner
/// reports these files and leaves them unchanged (the chat can be deleted instead). The 2.x layout
/// (`session_<...>/meta.json` and `messages.jsonl`, the logs of 0.1.1 users) has no digests and stays redactable.
///
/// Fails towards "sealed": the path is compared as written and in canonical form (a folder configured through a
/// link, or `\\?\C:\...` on Windows, as in `inside_log_folder`), and a side that cannot be resolved counts as sealed.
pub(crate) fn sealed(folder: &Path, path: &Path) -> bool {
    if unified_store(folder, path).is_some() {
        return true;
    }
    match (folder.canonicalize(), path.canonicalize()) {
        (Ok(folder), Ok(path)) => unified_store(&folder, &path).is_some(),
        _ => true,
    }
}

/// True for Vibe 3.0's chat index, `<folder>/unified/.session-index.json`. It is not checked: it is "a rebuildable
/// metadata catalogue" (`mistralai_vibe_local_harness/vibe/_session_catalog.py`) whose entries, one per chat,
/// Vibe derives from each chat's own store and re-derives whenever that store changes; Vibe drops a deleted chat's
/// row only at its next session/list, which Vibe Work calls right after every delete. So any value in it is also in
/// a store that is checked and reported as that chat's finding (deleting the chat removes both); checked itself, it
/// has no single owner and would only show as a file that could not be checked.
pub(crate) fn derived_index(folder: &Path, path: &Path) -> bool {
    path.strip_prefix(folder)
        .is_ok_and(|relative| relative == Path::new("unified").join(".session-index.json"))
}

/// Which chat a session log file in the scanned `folder` belongs to, and its title, from Vibe's own metadata only
/// (never from the text the scan matched). Layouts, in this order:
/// - a Vibe 3.0 unified store, `<folder>/unified/<id>/` ([`unified_store`]): the message text lies up to two
///   folders below the store's `meta.json` (`journal/`, `generations/<n>/`, `chunks/`), so that `meta.json` names
///   the owner, and only when it names the folder's own id (a stray or copied `meta.json` never hands one chat's
///   files to another); inside the store a file's own `session_id` (as in
///   `generations/<n>/projection-state.json`) never decides;
/// - a `meta.json` beside the file: Vibe 2.x (`session_<...>/meta.json` + `messages.jsonl`, 2.26.1 included);
/// - a legacy single JSON document (Vibe 1.x) carrying the fields itself or under `metadata`.
///
/// `None` (no owner) makes a hit a failed file, never a finding, so no redaction is ever authorised on a guess.
pub(crate) fn chat(folder: &Path, path: &Path) -> Option<(String, String)> {
    let meta = path.parent()?.join("meta.json");
    let (source, expected) = if let Some(root) = unified_store(folder, path) {
        let id = root.file_name()?.to_str()?.to_owned();
        (root.join("meta.json"), Some(id))
    } else if meta.is_file() {
        (meta, None)
    } else if path.extension()?.to_str()? == "json" {
        (path.to_path_buf(), None)
    } else {
        return None;
    };
    let value: LogOwner = serde_json::from_reader(std::fs::File::open(source).ok()?).ok()?;
    let mut value = value;
    if let Some(metadata) = value.metadata.take() {
        value = *metadata;
    }
    let raw = value.session_id.as_deref()?;
    if expected.is_some_and(|id| id != raw) {
        return None;
    }
    let id = clean(raw);
    if id.is_empty() {
        return None;
    }
    let title = clean(value.title.as_deref().unwrap_or("Untitled chat"));
    Some((id, title))
}
