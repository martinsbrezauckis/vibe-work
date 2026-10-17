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
    pub retry_auto: bool,
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
    pub fn changed(&self, path: &Path, stamp: &Stamp, fingerprint: &str, auto: bool) -> bool {
        self.files.get(path).is_none_or(|old| {
            old.stamp != *stamp || old.fingerprint != fingerprint || (auto && old.retry_auto)
        })
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

/// Read only ownership fields with serde; other metadata and legacy message arrays are skipped.
#[derive(Deserialize)]
struct LogOwner {
    session_id: Option<String>,
    title: Option<String>,
    metadata: Option<Box<LogOwner>>,
}

/// Vibe metadata establishes ownership; a legacy JSON document carries it under `metadata`.
pub(crate) fn chat(path: &Path) -> Option<(String, String)> {
    let meta = path.parent()?.join("meta.json");
    let source = if meta.is_file() {
        meta
    } else if path.extension()?.to_str()? == "json" {
        path.to_path_buf()
    } else {
        return None;
    };
    let value: LogOwner = serde_json::from_reader(std::fs::File::open(source).ok()?).ok()?;
    let mut value = value;
    if let Some(metadata) = value.metadata.take() {
        value = *metadata;
    }
    let id = clean(value.session_id.as_deref()?);
    if id.is_empty() {
        return None;
    }
    let title = clean(value.title.as_deref().unwrap_or("Untitled chat"));
    Some((id, title))
}
