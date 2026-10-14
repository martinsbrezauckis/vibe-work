//! Direct access to a local vault repository on disk, in the layout of ShellX Vault's store
//! (`third_party/shellx-vault/crates/vault-server/src/store.rs` @ the pin): `blobs/<2 hex>/<64 hex>`
//! (sealed chunks), `manifests/<generation, 8 digits>` (sealed snapshots, dense from 1, optional
//! `FIRST` marker after a prune), `keyfile.json` (the published keyfile).
//!
//! Why our own access instead of the upstream embedded server: the upstream store keeps
//! the head in process memory and publishes a generation by rename-over (`store.rs:245,816-843,
//! 1000-1020`), so two processes on one folder can overwrite each other's commit (Vault finding V-2).
//! Here every commit:
//! 1. takes an OS file lock on `commit.lock` in the repository (other Vibe Work processes wait);
//! 2. re-reads the head from disk and compares it with the head the change was built on
//!    (compare-before-write); a moved head is reported, never overwritten;
//! 3. publishes the next generation create-new (hard link of a fully written, synced temp file), so
//!    even a writer that ignores our lock (ShellX, the Vault app) can never be overwritten by us.
//!
//! The lock file is the only file we add; the upstream store never reads unknown files at the
//! repository root.
//!
//! Depends on: vault-core (ids), blake3, std, `chain` (the head guard a vault's sessions carry). Called by: `items`,
//! `vault`, `chain`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::chain::ChainGuard;
use crate::error::VaultError;

/// The current head: generation, identity (BLAKE3 of the sealed bytes) and the sealed manifest.
#[derive(Clone)]
pub struct Head {
    /// Generation number (1 = first commit).
    pub generation: u64,
    /// `Snapshot::sealed_id` of `sealed`.
    pub id: [u8; 32],
    /// The sealed manifest bytes.
    pub sealed: Vec<u8>,
}

/// Result of [`LocalRepo::commit_if_head`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    /// Published as this generation.
    Committed(u64),
    /// The head on disk is not the one the change was built on; nothing was written.
    HeadMoved,
}

/// The held commit lock; released when dropped (closing the file releases the OS lock).
#[derive(Debug)]
pub struct CommitLock {
    // Held only for its drop: closing the file releases the OS lock.
    _file: File,
}

/// One repository folder (`<data dir>/repos/<name>`).
#[derive(Debug, Clone)]
pub struct LocalRepo {
    dir: PathBuf,
    /// the head guard of the vault this repository belongs to (`None`: unguarded, e.g. while creating).
    chain: Option<Arc<ChainGuard>>,
}

const SUBDIRS: [&str; 6] = ["blobs", "manifests", "trash", "shares", "drops", "deposits"];
const LOCK_FILE: &str = "commit.lock";

fn valid_blob_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl LocalRepo {
    /// The repository at `dir` (nothing is read or created yet).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            chain: None,
        }
    }

    /// The same repository, its head reads checked by `chain` (`chain`).
    #[must_use]
    pub fn guarded(mut self, chain: Arc<ChainGuard>) -> Self {
        self.chain = Some(chain);
        self
    }

    /// The head guard, if this repository has one.
    pub(crate) fn chain(&self) -> Option<&ChainGuard> {
        self.chain.as_deref()
    }

    /// The repository folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Creates the folders the upstream store creates on open (`store.rs:268-279`), user-only.
    ///
    /// # Errors
    /// [`VaultError::Io`].
    pub fn init(&self) -> Result<(), VaultError> {
        create_private_dir_all(&self.dir)?;
        for sub in SUBDIRS {
            create_private_dir_all(&self.dir.join(sub))?;
        }
        Ok(())
    }

    /// True when the repository holds at least the manifests folder.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.dir.join("manifests").is_dir()
    }

    fn manifests(&self) -> PathBuf {
        self.dir.join("manifests")
    }

    fn manifest_path(&self, generation: u64) -> PathBuf {
        self.manifests().join(format!("{generation:08}"))
    }

    /// The highest generation on disk, the way the upstream store recovers its head
    /// (`store.rs:281-322`): every file whose name parses as a number counts, `FIRST` does not.
    ///
    /// # Errors
    /// [`VaultError::Io`].
    pub fn head(&self) -> Result<Option<Head>, VaultError> {
        let dir = self.manifests();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(VaultError::io("read the manifest list", &dir, error)),
        };
        let mut max = 0u64;
        for entry in entries {
            let entry = entry.map_err(|e| VaultError::io("read the manifest list", &dir, e))?;
            if let Some(generation) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u64>().ok())
            {
                max = max.max(generation);
            }
        }
        if max == 0 {
            return Ok(None);
        }
        let path = self.manifest_path(max);
        let sealed =
            std::fs::read(&path).map_err(|e| VaultError::io("read a manifest", &path, e))?;
        Ok(Some(Head {
            generation: max,
            id: *blake3::hash(&sealed).as_bytes(),
            sealed,
        }))
    }

    /// The sealed manifest of `generation` (`None` when there is no such file).
    ///
    /// # Errors
    /// [`VaultError::Io`].
    pub fn manifest(&self, generation: u64) -> Result<Option<Vec<u8>>, VaultError> {
        let path = self.manifest_path(generation);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(VaultError::io("read a manifest", &path, error)),
        }
    }

    /// A sealed chunk by its hex id.
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] for a malformed id, [`VaultError::Io`] (incl. a missing blob).
    pub fn blob(&self, id_hex: &str) -> Result<Vec<u8>, VaultError> {
        let path = self.blob_path(id_hex)?;
        std::fs::read(&path).map_err(|e| VaultError::io("read a sealed chunk", &path, e))
    }

    fn blob_path(&self, id_hex: &str) -> Result<PathBuf, VaultError> {
        if !valid_blob_id(id_hex) {
            return Err(VaultError::Corrupt("malformed chunk id".to_owned()));
        }
        Ok(self.dir.join("blobs").join(&id_hex[..2]).join(id_hex))
    }

    /// Stores a sealed chunk (content-addressed, so an existing one is kept as is).
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] for a malformed id, [`VaultError::Io`].
    pub fn put_blob(&self, id_hex: &str, sealed: &[u8]) -> Result<(), VaultError> {
        let path = self.blob_path(id_hex)?;
        if path.exists() {
            return Ok(());
        }
        publish_new(&path, sealed).map(|_| ())
    }

    /// Writes `keyfile.json` (the published keyfile ShellX keeps next to its data) unless one exists.
    ///
    /// # Errors
    /// [`VaultError::Io`].
    pub fn publish_keyfile(&self, json: &[u8]) -> Result<(), VaultError> {
        publish_new(&self.dir.join("keyfile.json"), json).map(|_| ())
    }

    /// Publishes `sealed` as the next generation if the head on disk is still `expected`
    /// (`None` = empty repository). See the module header for the three steps.
    ///
    /// # Errors
    /// [`VaultError::Io`]; a moved head is `Ok(CommitOutcome::HeadMoved)`, not an error.
    pub fn commit_if_head(
        &self,
        expected: Option<&Head>,
        sealed: &[u8],
    ) -> Result<CommitOutcome, VaultError> {
        let lock = self.lock()?;
        self.commit_if_head_locked(&lock, expected, sealed)
    }

    /// As [`LocalRepo::commit_if_head`], for a caller that already holds [`LocalRepo::lock`]
    /// (a whole read-modify-write cycle under one lock, so this app's own writers never race).
    ///
    /// # Errors
    /// [`VaultError::Io`].
    pub fn commit_if_head_locked(
        &self,
        _lock: &CommitLock,
        expected: Option<&Head>,
        sealed: &[u8],
    ) -> Result<CommitOutcome, VaultError> {
        let current = self.head()?;
        let same = match (expected, &current) {
            (None, None) => true,
            (Some(a), Some(b)) => a.generation == b.generation && a.id == b.id,
            _ => false,
        };
        if !same {
            return Ok(CommitOutcome::HeadMoved);
        }
        let next = current.map_or(1, |h| h.generation + 1);
        if publish_new(&self.manifest_path(next), sealed)? {
            Ok(CommitOutcome::Committed(next))
        } else {
            Ok(CommitOutcome::HeadMoved)
        }
    }

    /// Takes the repository's commit lock (blocks while another Vibe Work writer holds it). Writers
    /// that do not know this lock (ShellX, the Vault app) are still never overwritten: see the
    /// create-new publication in the module header.
    ///
    /// # Errors
    /// [`VaultError::Io`].
    pub fn lock(&self) -> Result<CommitLock, VaultError> {
        let path = self.dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| VaultError::io("open the commit lock", &path, e))?;
        file.lock()
            .map_err(|e| VaultError::io("take the commit lock", &path, e))?;
        Ok(CommitLock { _file: file })
    }
}

/// Creates a folder (and parents) and makes the last one user-only on Unix.
///
/// # Errors
/// [`VaultError::Io`].
pub(crate) fn create_private_dir_all(dir: &Path) -> Result<(), VaultError> {
    std::fs::create_dir_all(dir).map_err(|e| VaultError::io("create a folder", dir, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| VaultError::io("make a folder user-only", dir, e))?;
    }
    Ok(())
}

/// Writes a user-only temp file next to `path`, syncs it, and links it to `path` only if `path`
/// does not exist. Returns false when `path` already existed (nothing replaced).
fn publish_new(path: &Path, bytes: &[u8]) -> Result<bool, VaultError> {
    let dir = path
        .parent()
        .ok_or_else(|| VaultError::Corrupt("storage path without a folder".to_owned()))?;
    std::fs::create_dir_all(dir).map_err(|e| VaultError::io("create a folder", dir, e))?;
    let tmp = dir.join(format!(
        ".tmp-{}",
        hex::encode(vault_core::random_bytes::<8>())
    ));
    let result = write_synced(&tmp, bytes).and_then(|()| match std::fs::hard_link(&tmp, path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(VaultError::io("publish a file", path, error)),
    });
    let _ = std::fs::remove_file(&tmp);
    if matches!(result, Ok(true)) {
        sync_dir(dir);
    }
    result
}

/// Creates `path` new (user-only on Unix), writes and syncs it.
pub(crate) fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| VaultError::io("create a file", path, e))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| VaultError::io("write a file", path, e))
}

/// Best-effort folder sync so a published name survives a crash (Unix; Windows cannot open a
/// folder as a file, the same limit as upstream `store.rs:1027-1033`).
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(file) = File::open(dir) {
        let _ = file.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}
