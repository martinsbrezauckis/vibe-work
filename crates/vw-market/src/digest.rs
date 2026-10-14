//! Hashes: SHA-256 hex (vendor files, runtime archives, receipts) and npm's SRI `sha512-<base64>`.
//!
//! Depends on: sha2, base64. Called by: `catalog_rules`, `install`, `store`.
//! Invariant: comparisons are on the decoded digests, so letter case or base64 padding differences in the catalog
//! cannot make a wrong file pass.

use base64::Engine as _;
use sha2::{Digest as _, Sha256, Sha512};

/// SHA-256 of `bytes`, lowercase hex.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The 64-byte digest inside an SRI `sha512-<base64>` string, if well formed.
#[must_use]
pub fn parse_sri_sha512(integrity: &str) -> Option<Vec<u8>> {
    let encoded = integrity.strip_prefix("sha512-")?;
    let digest = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    (digest.len() == 64).then_some(digest)
}

/// True when `bytes` match the SRI `sha512-<base64>` string.
#[must_use]
pub fn matches_sri_sha512(bytes: &[u8], integrity: &str) -> bool {
    parse_sri_sha512(integrity)
        .is_some_and(|want| Sha512::digest(bytes).as_slice() == want.as_slice())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}
