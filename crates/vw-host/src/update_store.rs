//! The downloaded update on disk: `<data>/updates/package.bin` plus `pending.json` describing it.
//! One package at a time. The record is written only after the package passed signature verification, and
//! the package is verified again whenever it is loaded, so a file that changed on disk is never offered.
//!
//! Invariants:
//! - A partial download is `package.bin.part`; it is deleted at start (downloads restart, there is no resume).
//! - `pending.json` is renamed into place after `package.bin`, so a record never describes a missing file.
//! - [`UpdateStore::load`] returns a record only if the file hashes to the recorded sha256, the minisign signature
//!   verifies with the build's key for the recorded version ([`crate::update_verify`]) and the version is newer than
//!   the running one. Anything else deletes both files.
//! - Nothing here is secret (a public, signed installer and its metadata).
//!
//! Depends on: serde, sha2, semver, [`crate::update_verify`]. Called by: [`crate::update_flow`].

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::update_verify::verify_package;

/// What `pending.json` records about the downloaded package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// The announced (and signed) version.
    pub version: String,
    /// Release notes from the manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The announced signature (base64 minisign text), kept to verify again and to match a later announcement.
    pub signature: String,
    /// Hex sha256 of `package.bin`.
    pub sha256: String,
    /// Size of `package.bin` in bytes.
    pub size: u64,
}

/// The `updates` folder of the data directory.
#[derive(Debug, Clone)]
pub struct UpdateStore {
    dir: PathBuf,
}

fn hex_sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

impl UpdateStore {
    /// The store under `data_dir/updates` (created on the first save).
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("updates"),
        }
    }

    fn package(&self) -> PathBuf {
        self.dir.join("package.bin")
    }

    fn record(&self) -> PathBuf {
        self.dir.join("pending.json")
    }

    /// Deletes both files and any partial download. Missing files are fine.
    pub fn clear(&self) {
        for path in [
            self.package(),
            self.record(),
            self.dir.join("package.bin.part"),
            self.dir.join("pending.json.tmp"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Stores a verified package and its record (package first, record last).
    ///
    /// # Errors
    /// The I/O error text.
    pub fn save(
        &self,
        version: &str,
        notes: Option<&str>,
        signature: &str,
        package: &[u8],
    ) -> Result<Pending, String> {
        let io = |e: std::io::Error| format!("cannot store the update: {e}");
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let part = self.dir.join("package.bin.part");
        let mut file = std::fs::File::create(&part).map_err(io)?;
        file.write_all(package).map_err(io)?;
        file.sync_all().map_err(io)?;
        drop(file);
        // The old record must not point at the new file while it is being swapped in.
        let _ = std::fs::remove_file(self.record());
        std::fs::rename(&part, self.package()).map_err(io)?;
        let pending = Pending {
            version: version.to_owned(),
            notes: notes.map(str::to_owned),
            signature: signature.to_owned(),
            sha256: hex_sha256(package),
            size: package.len() as u64,
        };
        let temp = self.dir.join("pending.json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(&pending)
                .map_err(std::io::Error::other)
                .map_err(io)?,
        )
        .map_err(io)?;
        std::fs::rename(&temp, self.record()).map_err(io)?;
        Ok(pending)
    }

    /// Reads the package of `pending` and verifies it again (sha256, then signature).
    ///
    /// # Errors
    /// Why it cannot be used: unreadable, changed on disk, or not verifying.
    pub fn read_verified(&self, pending: &Pending, pubkey: &str) -> Result<Vec<u8>, String> {
        let bytes = std::fs::read(self.package())
            .map_err(|e| format!("the downloaded update cannot be read: {e}"))?;
        if hex_sha256(&bytes) != pending.sha256 {
            return Err("the downloaded update changed on disk".to_owned());
        }
        verify_package(&bytes, &pending.signature, pubkey, &pending.version)?;
        Ok(bytes)
    }

    /// The stored update, if there is one that is still good for `current` (see the module invariants); stale or
    /// broken files are deleted. Partial downloads are always deleted.
    #[must_use]
    pub fn load(&self, pubkey: &str, current: &str) -> Option<Pending> {
        let _ = std::fs::remove_file(self.dir.join("package.bin.part"));
        let text = std::fs::read(self.record()).ok()?;
        let Ok(pending) = serde_json::from_slice::<Pending>(&text) else {
            self.clear();
            return None;
        };
        let newer = match (
            semver::Version::parse(&pending.version),
            semver::Version::parse(current),
        ) {
            (Ok(offered), Ok(running)) => offered > running,
            _ => false,
        };
        if newer && self.read_verified(&pending, pubkey).is_ok() {
            Some(pending)
        } else {
            self.clear();
            None
        }
    }
}
