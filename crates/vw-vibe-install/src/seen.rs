//! The supported-Vibe manifest's rollback guard: the highest manifest `sequence` ever accepted on this
//! machine, kept in `<vibe root>/support-seen.json`.
//!
//! **Fail closed, both ways.**
//! - Reading: only a file that does not exist means "first use" (sequence 0). A file that exists but cannot be
//!   read, is not a regular file, or does not parse is [`SupportError::State`]: the check gives no offer instead of
//!   treating a damaged record as "nothing seen" (which would let any older signed manifest through).
//! - Writing: a higher sequence is accepted only once it is durably on disk. When it cannot be saved the check fails
//!   with [`SupportError::State`] and nothing is offered, so the app never acts on a manifest whose freshness it
//!   could not record (after a restart an older signed list would otherwise pass again).
//!
//! **Durable atomic write.** The new record goes to a uniquely named temporary file in the same folder, created
//! exclusively (`create_new`: an existing file or a planted symlink at that name fails instead of being followed),
//! written and `fsync`-ed, then renamed over the record; on Unix the folder is `fsync`-ed afterwards so the rename
//! itself survives a power loss. Windows has no folder sync through the standard library; there the rename
//! (`MoveFileExW` with replace) is atomic but its durability is left to NTFS's metadata journal. A crash leaves the
//! old record or the new one, never half of one; a stray temporary file is ignored by [`SeenSequence::get`].
//!
//! Recovery: the error names the file. A user (or support) who decides the damaged record is not an attack can
//! delete it; the next check then starts from sequence 0, exactly like a fresh install.
//!
//! Depends on: serde, getrandom (temporary name). Called by: [`crate::update`] (through [`crate::support`]'s
//! re-export), the integration tests of the supported-Vibe manifest (rollback-record cases).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::support::{Manifest, SupportError};

/// The record's file name under the Vibe root.
pub const SEEN_FILE: &str = "support-seen.json";
/// Largest record accepted (a real one is ~20 bytes); anything bigger is damage, not a record.
const MAX_SEEN_BYTES: u64 = 4 * 1024;

/// The highest manifest `sequence` accepted on this machine (`<vibe root>/support-seen.json`), the rollback guard.
#[derive(Debug, Clone)]
pub struct SeenSequence {
    path: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeenFile {
    sequence: u64,
}

fn state_error(path: &Path, what: &str, detail: impl std::fmt::Display) -> SupportError {
    SupportError::State(format!("{what} {}: {detail}", path.display()))
}

impl SeenSequence {
    /// The guard stored under `root`.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join(SEEN_FILE),
        }
    }

    /// The record's path (shown in errors; used by tests).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The highest sequence accepted so far: 0 only when the record does not exist yet.
    ///
    /// # Errors
    /// [`SupportError::State`] when the record exists but cannot be read (permissions, not a regular file, too
    /// large) or does not parse. Never "0" for a damaged record: that would reopen every older signed manifest.
    pub fn get(&self) -> Result<u64, SupportError> {
        let meta = match std::fs::symlink_metadata(&self.path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(state_error(&self.path, "cannot inspect", error)),
        };
        if !meta.is_file() {
            return Err(state_error(
                &self.path,
                "cannot use",
                "not a regular file (delete it to start over)",
            ));
        }
        if meta.len() > MAX_SEEN_BYTES {
            return Err(state_error(
                &self.path,
                "cannot use",
                format!("{} bytes, not a sequence record", meta.len()),
            ));
        }
        let bytes = std::fs::read(&self.path)
            .map_err(|error| state_error(&self.path, "cannot read", error))?;
        serde_json::from_slice::<SeenFile>(&bytes)
            .map(|f| f.sequence)
            .map_err(|error| {
                state_error(
                    &self.path,
                    "damaged rollback record",
                    format!("{error} (delete it to start over)"),
                )
            })
    }

    /// Accepts `manifest` when its sequence is not lower than the highest seen, and durably records a higher one
    /// before returning.
    ///
    /// # Errors
    /// [`SupportError::Rollback`] for an older manifest; [`SupportError::State`] when the record cannot be read
    /// ([`Self::get`]) or a higher sequence cannot be saved (the caller then offers nothing).
    pub fn accept(&self, manifest: &Manifest) -> Result<(), SupportError> {
        let seen = self.get()?;
        if manifest.sequence < seen {
            return Err(SupportError::Rollback {
                found: manifest.sequence,
                seen,
            });
        }
        if manifest.sequence > seen {
            self.write(manifest.sequence)
                .map_err(|error| state_error(&self.path, "cannot save", error))?;
        }
        Ok(())
    }

    /// Writes `sequence` atomically and durably (see the module header).
    fn write(&self, sequence: u64) -> std::io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| std::io::Error::other("the record has no folder"))?;
        std::fs::create_dir_all(parent)?;
        let text = serde_json::to_vec(&SeenFile { sequence }).map_err(std::io::Error::other)?;
        let temp = parent.join(format!("{SEEN_FILE}.{}.tmp", temp_suffix()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .and_then(|mut file| {
                file.write_all(&text)?;
                file.sync_all()
            })
            .and_then(|()| std::fs::rename(&temp, &self.path));
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
        sync_dir(parent)
    }
}

/// 64 random bits (hex) plus the process id: unique per write, and `create_new` refuses a collision anyway.
fn temp_suffix() -> String {
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes.copy_from_slice(&nanos.to_le_bytes()[..8]);
    }
    format!("{}-{}", std::process::id(), crate::pin::hex(&bytes))
}

/// Makes the rename durable: `fsync` of the folder on Unix.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Windows: no folder sync through the standard library (see the module header).
#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the Unix version"
)]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}
