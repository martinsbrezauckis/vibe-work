//! Secret ownership and value-free reports. Depends on zeroize and serde; used by the public API.

use std::{fmt, io::ErrorKind, path::PathBuf};

use serde::Serialize;
use zeroize::Zeroizing;

/// An owned secret, whose value is wiped on drop and never included in debug output.
pub struct Secret {
    pub(crate) id: String,
    pub(crate) value: Zeroizing<String>,
}

impl Secret {
    /// Transfer ownership of a value to the scanner. The caller should avoid retaining a copy.
    #[must_use]
    pub fn new(id: impl Into<String>, value: String) -> Self {
        Self {
            id: id.into(),
            value: Zeroizing::new(value),
        }
    }

    pub(crate) fn report(&self) -> SecretReport {
        let skipped = (self.value.chars().count() < 8).then_some(SecretSkipReason::TooShort);
        let prefix = ["github_pat_", "ghp_", "sk-"].into_iter().find(|prefix| {
            skipped.is_none() && self.value.len() > prefix.len() && self.value.starts_with(prefix)
        });
        SecretReport {
            id: self.id.clone(),
            masked_preview: format!("{}****", prefix.unwrap_or_default()),
            skipped,
            locations: Vec::new(),
        }
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("id", &self.id)
            .field("value", &"[redacted]")
            .finish()
    }
}

/// The representation found in the bytes of a log file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Encoding {
    /// Unescaped UTF-8 bytes.
    AsIs,
    /// The contents of a JSON-serialized string, without its outer quotes.
    JsonEscaped,
    /// Standard or URL-safe base64, with or without padding.
    Base64,
    /// RFC 3986 percent encoding or HTML form encoding (spaces as '+'), with uppercase hex.
    UrlEncoded,
}

/// Why a supplied value was not scanned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SecretSkipReason {
    /// Fewer than eight Unicode characters produce too many false matches.
    TooShort,
}

/// A match in the original file; no value or surrounding text is retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    /// The file containing the match.
    pub file: PathBuf,
    /// One-based physical line containing the first matched byte.
    pub line: u64,
    /// Zero-based byte offset from the beginning of the original file.
    pub byte_offset: u64,
    /// Number of original bytes matched.
    pub byte_length: u64,
    /// The representation that matched. Identical forms are reported only once.
    pub encoding: Encoding,
}

/// All locations associated with one caller-supplied secret identifier.
#[derive(Debug, Serialize)]
pub struct SecretReport {
    /// Opaque caller-supplied identifier; callers must not use secret values as identifiers.
    pub id: String,
    /// A recognized public token prefix followed by stars, or just stars.
    pub masked_preview: String,
    /// A skipped value remains visible in the report with its reason.
    pub skipped: Option<SecretSkipReason>,
    /// Every occurrence in each scanned file.
    pub locations: Vec<Location>,
}

/// Why a file could not be processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FileSkipReason {
    /// Content no longer matches the scan's fingerprint.
    ChangedSinceScan,
    /// Replacing the ranges would leave invalid JSON or JSONL; the source is untouched.
    InvalidJson,
    /// The file system refused a read, temporary write, or replacement.
    Io,
}

/// A skipped file and its reason; never includes file contents or parser error text.
#[derive(Debug, Serialize)]
pub struct FileIssue {
    /// The affected file.
    pub file: PathBuf,
    /// Why processing stopped for this file.
    pub reason: FileSkipReason,
}

/// A value-free report that can later authorize redaction without reopening the vault.
#[derive(Debug, Serialize)]
pub struct ScanReport {
    /// One result per supplied secret, including skipped and unmatched secrets.
    pub secrets: Vec<SecretReport>,
    /// Files that could not be scanned.
    pub skipped_files: Vec<FileIssue>,
    #[serde(skip)]
    pub(crate) files: Vec<Snapshot>,
    #[serde(skip)]
    pub(crate) format_guard: Option<fn(&std::path::Path, &std::path::Path) -> bool>,
}

impl ScanReport {
    /// Add a caller's format check before atomic replacement. The guard must validate JSON/JSONL
    /// syntax as well as the caller's schema. It receives original and proposed file paths only.
    #[must_use]
    pub fn with_format_guard(
        mut self,
        guard: fn(&std::path::Path, &std::path::Path) -> bool,
    ) -> Self {
        self.format_guard = Some(guard);
        self
    }

    /// Matching places after merging duplicate and overlapping matches, as redaction does.
    #[must_use]
    pub fn place_count(&self) -> usize {
        self.files
            .iter()
            .map(|file| {
                let mut ranges = file.ranges.clone();
                ranges.sort_by_key(|range| range.start);
                crate::redact::merge_ranges(ranges).len()
            })
            .sum()
    }
    /// Retain redaction authority only for the named secrets. Snapshots keep their original content hash.
    /// This does not accept new byte ranges or authority from a serialized report.
    #[must_use]
    pub fn select_secrets(&self, ids: &std::collections::BTreeSet<String>) -> Self {
        let secrets: Vec<_> = self
            .secrets
            .iter()
            .filter(|secret| ids.contains(&secret.id))
            .map(|secret| SecretReport {
                id: secret.id.clone(),
                masked_preview: secret.masked_preview.clone(),
                skipped: secret.skipped,
                locations: secret.locations.clone(),
            })
            .collect();
        let files = self
            .files
            .iter()
            .filter_map(|snapshot| {
                let ranges: Vec<_> = secrets
                    .iter()
                    .flat_map(|secret| &secret.locations)
                    .filter(|location| location.file == snapshot.path)
                    .map(|location| {
                        location.byte_offset..location.byte_offset + location.byte_length
                    })
                    .collect();
                (!ranges.is_empty()).then(|| Snapshot {
                    path: snapshot.path.clone(),
                    hash: snapshot.hash,
                    ranges,
                })
            })
            .collect();
        Self {
            secrets,
            skipped_files: Vec::new(),
            files,
            format_guard: self.format_guard,
        }
    }
}

/// The outcome of independently redacting each affected file.
#[derive(Debug, Default, Serialize)]
pub struct RedactionReport {
    /// Files replaced successfully.
    pub redacted_files: Vec<PathBuf>,
    /// Files left untouched, with the reason.
    pub skipped_files: Vec<FileIssue>,
}

#[derive(Debug)]
pub(crate) struct Snapshot {
    pub path: PathBuf,
    pub hash: [u8; 32],
    pub ranges: Vec<std::ops::Range<u64>>,
}

/// An error that prevents starting or traversing a scan; contains no secret data.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The supplied folder could not be read.
    #[error("could not traverse the session folder: {0:?}")]
    Folder(ErrorKind),
    /// An in-memory encoding could not be prepared.
    #[error("could not encode a supplied secret")]
    Encoding,
}
