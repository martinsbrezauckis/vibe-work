//! Authenticated settings and certificate snapshots for the core's key-bearing clients.
//! Depends on HMAC-SHA256 and serde; launch and Settings read only records verified with the vault's key.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::{MAX_FILE_BYTES, NetworkSettings, PreparedSettings};

const VERSION: u32 = 1;
const DOMAIN: &[u8] = b"vibe-work-network-settings\0";

/// A 32-byte key from the Mistral key's protected store. Never serialized or exposed by Debug.
pub struct NetworkMacKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for NetworkMacKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NetworkMacKey(<redacted>)")
    }
}

impl NetworkMacKey {
    /// Wraps operating-system random bytes read from, or about to be stored in, the vault.
    #[must_use]
    pub fn new(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }

    fn mac(&self, version: u32, bytes: &[u8]) -> io::Result<Hmac<Sha256>> {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.0.as_ref()).map_err(io::Error::other)?;
        mac.update(DOMAIN);
        mac.update(&version.to_be_bytes());
        mac.update(bytes);
        Ok(mac)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    // Kept as text so verification covers the exact persisted payload bytes, before parsing them.
    settings: String,
    mac: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    values: BTreeMap<String, String>,
    certificate_file: Option<Vec<u8>>,
    certificate_dir: Option<Vec<Vec<u8>>>,
}

/// Settings whose provenance was checked; an unusable record yields defaults and one security notice.
#[derive(Debug, Default)]
pub struct LoadedSettings {
    /// The verified settings, or defaults if verification failed.
    pub settings: NetworkSettings,
    /// True if a record existed but could not be authenticated or read.
    pub rejected: bool,
}

/// Reads and verifies the record before exposing any settings or certificates.
/// Missing files and legacy maps without custom proxy/CA need no vault. Every authenticated envelope
/// requires the current key, including one saved when all custom settings were removed.
#[must_use]
pub fn read_settings(path: &Path, key: Option<&NetworkMacKey>) -> LoadedSettings {
    let read = || -> Option<NetworkSettings> {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return None;
        }
        let bytes = std::fs::read(path).ok()?;
        if let Ok(values) = serde_json::from_slice::<BTreeMap<String, String>>(&bytes) {
            let settings = NetworkSettings::from_lookup(|name| values.get(name).cloned());
            return (!settings.has_custom()).then_some(settings);
        }
        let record: Record = serde_json::from_slice(&bytes).ok()?;
        if record.version != VERSION {
            return None;
        }
        key?.mac(record.version, record.settings.as_bytes())
            .ok()?
            .verify_slice(&record.mac)
            .ok()?;
        let payload: Payload = serde_json::from_str(&record.settings).ok()?;
        let mut settings = NetworkSettings::from_lookup(|name| payload.values.get(name).cloned());
        settings.file_certificates = payload.certificate_file;
        settings.dir_certificates = payload.certificate_dir;
        Some(settings)
    };
    match read() {
        Some(settings) => LoadedSettings {
            settings,
            rejected: false,
        },
        None => LoadedSettings {
            settings: NetworkSettings::default(),
            rejected: !matches!(std::fs::metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound),
        },
    }
}

pub(super) fn payload_text(settings: &NetworkSettings) -> io::Result<String> {
    let text = serde_json::to_string(&Payload {
        values: super::KEYS
            .iter()
            .filter_map(|name| settings.get(name).map(|value| ((*name).to_owned(), value)))
            .collect(),
        certificate_file: settings.file_certificates.clone(),
        certificate_dir: settings.dir_certificates.clone(),
    })?;
    // Check the largest possible MAC encoding before asking for consent or touching the protected key.
    record_text(&text, vec![255; 32])?;
    Ok(text)
}

fn record_text(settings: &str, mac: Vec<u8>) -> io::Result<String> {
    let text = serde_json::to_string_pretty(&Record {
        version: VERSION,
        settings: settings.to_owned(),
        mac,
    })?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other(
            "The network settings and certificates are too large to save.",
        ));
    }
    Ok(text)
}

/// Authenticates the immutable candidate after native Save confirmation, without filesystem reads.
///
/// # Errors
/// The vault key is unavailable or the record exceeds the file limit.
pub fn seal_settings(
    prepared: &PreparedSettings,
    key: Option<&NetworkMacKey>,
) -> io::Result<String> {
    let settings = prepared.settings();
    if key.is_none() && !settings.has_custom() {
        return Ok(settings.to_file_text());
    }
    let key = key.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unlock the vault and save the network settings again.",
        )
    })?;
    let bytes = &prepared.payload;
    let mac = key
        .mac(VERSION, bytes.as_bytes())?
        .finalize()
        .into_bytes()
        .to_vec();
    record_text(bytes, mac)
}
