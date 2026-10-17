//! Streaming secret detection and atomic redaction of a caller's session-log folder.
//!
//! Depends on standard encoders, memchr, `serde_json`, sha2, tempfile and zeroize.
//! Called by Rust clients and fixture tests; there is no vault or UI dependency.
//! Scans consume and wipe secret values. Reports contain identifiers and locations only.

mod patterns;
mod redact;
mod scan;
mod types;
mod validate;

pub use types::{
    Encoding, Error, FileIssue, FileSkipReason, Location, RedactionReport, ScanReport, Secret,
    SecretReport, SecretSkipReason,
};

/// Scan JSON and JSONL files recursively, without following symbolic links.
///
/// Consumes the values, including skipped values, and zeroizes them before returning.
/// Lines are one-based and byte offsets are zero-based in the original file.
/// Working memory depends on the supplied secrets and returned matches, not file size.
///
/// # Errors
/// Returns an error if the input folder cannot be traversed or a value cannot be encoded. Unreadable files
/// are recorded in the report so other sessions can still be scanned.
pub fn scan(folder: &std::path::Path, secrets: Vec<Secret>) -> Result<ScanReport, Error> {
    scan::scan(folder, secrets)
}

/// Scan selected regular JSON/JSONL files, using the same matching and snapshots as [`scan`].
/// Callers can schedule one file at a time and skip unchanged files. Symlinks are ignored.
///
/// # Errors
/// Returns an error if a supplied value cannot be encoded. File read failures are reported individually.
pub fn scan_files(
    files: Vec<std::path::PathBuf>,
    secrets: Vec<Secret>,
) -> Result<ScanReport, Error> {
    scan::scan_files(files.into_iter().map(Ok), secrets)
}

/// Redact all reported ranges using a new file and an atomic rename per affected file.
///
/// Each source's content hash must still match the scan. Invalid JSON/JSONL and changed
/// files are skipped. Callers should stop session writers while scanning and redacting.
#[must_use]
pub fn redact(report: &ScanReport) -> RedactionReport {
    redact::redact(report)
}
