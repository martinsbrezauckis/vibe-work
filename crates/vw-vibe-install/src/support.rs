//! The signed supported-Vibe manifest: which Vibe versions Vibe Work has tested, published beside the
//! app update (`latest.json`) on the same release channel and signed with the same minisign updater key. It lets us
//! approve a new Vibe after type and behavior checks, then publish a new manifest, without
//! rebuilding the app.
//!
//! Wire form (`vibe-support.json`, written by `tools/public/vibe-support/write_manifest.py`):
//! ```json
//! {"schema": 1, "sequence": 1760180000, "published": "2026-10-11T12:00:00Z", "pypiLatest": "2.27.0",
//!  "versions": [{"version": "2.26.2", "python": "3.12", "minVibeWork": "0.1.0", "maxVibeWork": null,
//!                "requirements": "vibe-2.26.2.requirements.txt", "requirementsSha256": "<hex>",
//!                "requirementsBytes": 80872, "packages": 101,
//!                "wheels": {"x86_64-unknown-linux-gnu": {"filename": "...", "bytes": 1, "sha256": "<hex>"}}}]}
//! ```
//! The signature is `vibe-support.json.sig`: base64 of the minisign signature text, exactly the encoding the Tauri
//! CLI writes for update packages (`tools/public/vibe-support/sign`).
//!
//! **Fail closed.** [`verify`] accepts a manifest only when:
//! - the Ed25519 signature verifies against the build's updater public key (the same key `latest.json` packages
//!   are checked with);
//! - the signature's trusted comment (covered by minisign's global signature) names `file:vibe-support.json`, so a
//!   signature made for an update package with the same key can never stand in for the manifest;
//! - the JSON parses as schema 1 and every entry is well formed (plain dotted versions, which become folder names;
//!   a plain requirements file name; lower-case hex SHA-256 values; semver app bounds).
//!
//! Anything else is a [`SupportError`] and means **no offer**. A replayed older manifest is refused by
//! [`SeenSequence`] (`crate::seen`): the highest `sequence` ever accepted is stored durably and a lower one is a
//! rollback; a record that cannot be read or saved is an error too ([`SupportError::State`]), never "sequence 0".
//!
//! [`evaluate`] turns an accepted manifest into what the update area shows: the newest version this app may
//! update to (only ever newer than the running one, never a downgrade), a newer Vibe on `PyPI` that has no support
//! yet (`pypiLatest`, never installable), or, for a user's own install, a version notice only.
//!
//! Depends on: minisign-verify, base64, semver, serde. Called by: [`crate::update`], the private tests.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

/// The manifest's file name on the channel (next to `latest.json`).
pub const MANIFEST_FILE: &str = "vibe-support.json";
/// The signature's file name on the channel.
pub const SIGNATURE_FILE: &str = "vibe-support.json.sig";
/// The schema this build reads.
pub const SCHEMA: u32 = 1;
/// Largest manifest accepted (a real one is a few kB).
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
/// Largest signature accepted (a real one is ~0.5 kB).
pub const MAX_SIGNATURE_BYTES: u64 = 8 * 1024;
/// Largest requirements file accepted (2.26.1's is 81 kB).
pub const MAX_REQUIREMENTS_BYTES: u64 = 4 * 1024 * 1024;

/// Why a manifest gives no offer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SupportError {
    /// The manifest or its signature could not be fetched (offline, missing file, too large).
    #[error("the supported-Vibe list could not be fetched: {0}")]
    Fetch(String),
    /// The key or signature does not decode, or the signature does not verify.
    #[error("the supported-Vibe list is not signed with this build's update key: {0}")]
    Signature(String),
    /// The signature is valid but was made for another file.
    #[error("the signature belongs to another file ({0}), not the supported-Vibe list")]
    OtherFile(String),
    /// The JSON does not parse or an entry is malformed.
    #[error("the supported-Vibe list is malformed: {0}")]
    Malformed(String),
    /// A schema this build does not read.
    #[error("the supported-Vibe list has schema {0}; this build reads schema {SCHEMA}")]
    Schema(u32),
    /// An older manifest than one already accepted (a replay).
    #[error(
        "an older supported-Vibe list was served (sequence {found}, already seen {seen}): ignored"
    )]
    Rollback {
        /// The served sequence.
        found: u64,
        /// The highest sequence accepted before.
        seen: u64,
    },
    /// The rollback record (`support-seen.json`) exists but cannot be read or parsed, or a newer sequence could not
    /// be saved. Fail closed: no offer until the record is readable and writable again.
    #[error("the supported-Vibe rollback record is not usable, so nothing is offered: {0}")]
    State(String),
}

/// One wheel of `mistral-vibe` for one target (shown in the consent; uv checks it through the requirements).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WheelInfo {
    /// The wheel's file name.
    pub filename: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// Its SHA-256 (lower-case hex).
    pub sha256: String,
}

/// One tested Vibe version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedVibe {
    /// The `mistral-vibe` version (plain dotted digits).
    pub version: String,
    /// The Python version request for uv (`3.12`).
    pub python: String,
    /// The oldest Vibe Work version it was tested with (semver, inclusive).
    pub min_vibe_work: String,
    /// The newest Vibe Work version it may run with (inclusive), `null` for no upper bound.
    #[serde(default)]
    pub max_vibe_work: Option<String>,
    /// The hash-locked requirements file's name on the channel (no path).
    pub requirements: String,
    /// Its SHA-256 (lower-case hex): checked before the file is used.
    pub requirements_sha256: String,
    /// Its size in bytes.
    pub requirements_bytes: u64,
    /// Packages it installs (Vibe and its dependencies).
    pub packages: u32,
    /// The `mistral-vibe` wheel per Rust target triple.
    pub wheels: BTreeMap<String, WheelInfo>,
}

/// An accepted manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Schema version ([`SCHEMA`]).
    pub schema: u32,
    /// Grows with every published manifest; a lower one than already seen is a rollback.
    pub sequence: u64,
    /// When it was published (RFC 3339, informational).
    #[serde(default)]
    pub published: String,
    /// The newest `mistral-vibe` on `PyPI` when it was published (may be newer than every tested version).
    #[serde(default)]
    pub pypi_latest: Option<String>,
    /// The tested versions.
    pub versions: Vec<SupportedVibe>,
}

/// Compares two plain dotted versions numerically (`2.26.10` > `2.26.9`; `2.26` = `2.26.0`). `None` when either
/// is not plain (digits and dots only).
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let parts = |v: &str| -> Option<Vec<u64>> {
        if v.is_empty() {
            return None;
        }
        v.split('.')
            .map(|p| {
                if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                    None
                } else {
                    p.parse::<u64>().ok()
                }
            })
            .collect()
    };
    let (mut x, mut y) = (parts(a)?, parts(b)?);
    let len = x.len().max(y.len());
    x.resize(len, 0);
    y.resize(len, 0);
    Some(x.cmp(&y))
}

fn newer(a: &str, b: &str) -> bool {
    compare_versions(a, b) == Some(Ordering::Greater)
}

fn is_hex64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn base64_text(input: &str) -> Result<String, SupportError> {
    let bytes = STANDARD
        .decode(input.trim())
        .map_err(|_| SupportError::Signature("not valid base64".to_owned()))?;
    String::from_utf8(bytes).map_err(|_| SupportError::Signature("not text".to_owned()))
}

fn check_entry(entry: &SupportedVibe) -> Result<(), String> {
    let name = &entry.version;
    if compare_versions(name, name).is_none() {
        return Err(format!("version {name:?} is not a plain version"));
    }
    if compare_versions(&entry.python, &entry.python).is_none() {
        return Err(format!(
            "{name}: python {:?} is not a version",
            entry.python
        ));
    }
    if semver::Version::parse(&entry.min_vibe_work).is_err() {
        return Err(format!("{name}: minVibeWork is not a version"));
    }
    if let Some(max) = &entry.max_vibe_work
        && semver::Version::parse(max).is_err()
    {
        return Err(format!("{name}: maxVibeWork is not a version"));
    }
    if !is_plain_file_name(&entry.requirements) {
        return Err(format!(
            "{name}: the requirements name is not a plain file name"
        ));
    }
    if !is_hex64(&entry.requirements_sha256) {
        return Err(format!("{name}: requirementsSha256 is not a SHA-256"));
    }
    if entry.requirements_bytes == 0 || entry.requirements_bytes > MAX_REQUIREMENTS_BYTES {
        return Err(format!("{name}: requirementsBytes out of range"));
    }
    for (target, wheel) in &entry.wheels {
        if !is_hex64(&wheel.sha256) || wheel.filename.is_empty() {
            return Err(format!("{name}: the {target} wheel is malformed"));
        }
    }
    Ok(())
}

/// Checks `bytes` (the manifest) against `signature` (base64 minisign text) and `pubkey` (the build's updater key,
/// base64 minisign text as in the Tauri config), then parses and validates it. Rollback is checked separately
/// ([`SeenSequence`]).
///
/// # Errors
/// [`SupportError::Signature`], [`SupportError::OtherFile`], [`SupportError::Malformed`],
/// [`SupportError::Schema`].
pub fn verify(bytes: &[u8], signature: &str, pubkey: &str) -> Result<Manifest, SupportError> {
    let key = PublicKey::decode(&base64_text(pubkey)?)
        .map_err(|_| SupportError::Signature("the update key in this build is not valid".into()))?;
    let signature = Signature::decode(&base64_text(signature)?)
        .map_err(|_| SupportError::Signature("the signature is not valid".into()))?;
    key.verify(bytes, &signature, false)
        .map_err(|_| SupportError::Signature("the signature does not match".into()))?;
    // Only after the signature verified is the trusted comment trustworthy.
    let file = signature
        .trusted_comment()
        .split('\t')
        .find_map(|pair| pair.strip_prefix("file:"))
        .unwrap_or("");
    if file != MANIFEST_FILE {
        return Err(SupportError::OtherFile(if file.is_empty() {
            "no file name".to_owned()
        } else {
            file.to_owned()
        }));
    }
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|e| SupportError::Malformed(e.to_string()))?;
    if manifest.schema != SCHEMA {
        return Err(SupportError::Schema(manifest.schema));
    }
    if let Some(latest) = &manifest.pypi_latest
        && compare_versions(latest, latest).is_none()
    {
        return Err(SupportError::Malformed(
            "pypiLatest is not a plain version".into(),
        ));
    }
    for entry in &manifest.versions {
        check_entry(entry).map_err(SupportError::Malformed)?;
    }
    Ok(manifest)
}

pub use crate::seen::SeenSequence;

/// The running Vibe, as the evaluation sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Running {
    /// Not installed yet (setup installs the pinned version; no update offer).
    None,
    /// Vibe Work's own install of this version.
    Private(String),
    /// The user's own install of this version ("use my existing Vibe").
    Existing(String),
}

/// What the update area shows after a check.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Evaluation {
    /// The newest tested version this app may update to (newer than the running one), with its wheel for this OS.
    pub available: Option<SupportedVibe>,
    /// A newer Vibe on `PyPI` that has no support yet (never installed).
    pub upcoming: Option<String>,
    /// For a user's own install: the version notice (no install is ever offered for it).
    pub mismatch: Option<String>,
}

fn applies(entry: &SupportedVibe, app: &semver::Version, target: &str) -> bool {
    let min_ok = semver::Version::parse(&entry.min_vibe_work).is_ok_and(|min| *app >= min);
    let max_ok = entry
        .max_vibe_work
        .as_deref()
        .is_none_or(|max| semver::Version::parse(max).is_ok_and(|max| *app <= max));
    min_ok && max_ok && entry.wheels.contains_key(target)
}

/// Turns `manifest` into the update area's content for this app (`app_version`, semver), this OS (`target`), the
/// running Vibe and the build's pinned version.
#[must_use]
pub fn evaluate(
    manifest: &Manifest,
    app_version: &str,
    target: &str,
    running: &Running,
    pinned: &str,
) -> Evaluation {
    let Ok(app) = semver::Version::parse(app_version) else {
        return Evaluation::default();
    };
    let mut usable: Vec<&SupportedVibe> = manifest
        .versions
        .iter()
        .filter(|e| applies(e, &app, target))
        .collect();
    usable.sort_by(|a, b| compare_versions(&a.version, &b.version).unwrap_or(Ordering::Equal));
    let newest_supported = usable.last().map(|e| e.version.clone());
    match running {
        Running::None => Evaluation::default(),
        Running::Private(current) => {
            let available = usable
                .iter()
                .rev()
                .find(|e| newer(&e.version, current))
                .map(|e| (*e).clone());
            let ceiling = available
                .as_ref()
                .map_or(current.as_str(), |e| e.version.as_str());
            let upcoming = manifest
                .pypi_latest
                .as_ref()
                .filter(|latest| newer(latest, ceiling))
                .filter(|latest| !usable.iter().any(|e| &e.version == *latest))
                .cloned();
            Evaluation {
                available,
                upcoming,
                mismatch: None,
            }
        }
        Running::Existing(current) => {
            let mut supported: Vec<String> = usable.iter().map(|e| e.version.clone()).collect();
            supported.push(pinned.to_owned());
            let best = newest_supported
                .filter(|n| newer(n, pinned))
                .unwrap_or_else(|| pinned.to_owned());
            let mismatch = if !supported.contains(current) {
                Some(format!(
                    "Your own Vibe is version {current}; Vibe Work is tested with Vibe {best}. Some features may \
                     not work as expected. Update your own install to {best}, or let Vibe Work use its own copy."
                ))
            } else if newer(&best, current) {
                Some(format!(
                    "Vibe {best} is now supported; your own Vibe is version {current}. Vibe Work does not change \
                     your own install: update it yourself when you are ready."
                ))
            } else {
                None
            };
            Evaluation {
                available: None,
                upcoming: None,
                mismatch,
            }
        }
    }
}
