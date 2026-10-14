//! Installer-independent private vault storage and verified relocation from the older location.
//!
//! Depends on std, serde and BLAKE3; called by `managed`. The profile lives in a sibling of
//! the identifier's app-data folder. Migration uses a same-filesystem rename, never a copy or
//! recursive delete. A durable manifest verifies every file and directory before selection
//! changes; failed verification moves the folder back, and interruption can be resumed.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use crate::VaultError;
use crate::repo::{create_private_dir_all, write_synced};

/// The installer deletes this container only after a separate explicit vault confirmation.
pub const PRIVATE_CONTAINER: &str = "Vibe Work Vault";

/// Resolves the private profile outside the app-data identifier folder on every OS.
///
/// Windows: `%APPDATA%/Vibe Work Vault/vault`; macOS: `~/Library/Application Support/`;
/// Linux: `$XDG_DATA_HOME` (or `~/.local/share`), followed by `Vibe Work Vault/vault`.
/// Explicit development/test app-data roots use their own sibling, without looking up a home.
///
/// # Errors
/// A relative path, filesystem root or path containing `..` is refused.
pub fn private_vault_dir(data_dir: &Path) -> Result<PathBuf, VaultError> {
    if !data_dir.is_absolute()
        || data_dir
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(VaultError::Corrupt(
            "app data must be an absolute folder".to_owned(),
        ));
    }
    let parent = data_dir
        .parent()
        .filter(|p| p != &data_dir)
        .ok_or_else(|| VaultError::Corrupt("app data must have a parent folder".to_owned()))?;
    if data_dir.file_name().is_none_or(|n| n == PRIVATE_CONTAINER) {
        return Err(VaultError::Corrupt(
            "app data overlaps the private vault container".to_owned(),
        ));
    }
    Ok(parent.join(PRIVATE_CONTAINER).join("vault"))
}

/// Rejects links and Windows junction/reparse points without following them.
pub(crate) fn check_path(path: &Path) -> Result<(), VaultError> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if is_link(&meta) => {
                return Err(VaultError::Corrupt(
                    "vault path contains a link or reparse point".to_owned(),
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(VaultError::io("check vault path", ancestor, e)),
        }
    }
    Ok(())
}

fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

type Manifest = BTreeMap<PathBuf, Option<String>>;

fn snapshot(root: &Path) -> Result<Manifest, VaultError> {
    let mut manifest = BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let path = root.join(&relative);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| VaultError::io("inspect vault move", &path, e))?;
        if is_link(&meta) {
            return Err(VaultError::Corrupt(
                "vault move refuses links and reparse points".to_owned(),
            ));
        }
        if meta.is_dir() {
            manifest.insert(relative.clone(), None);
            for entry in
                std::fs::read_dir(&path).map_err(|e| VaultError::io("list vault move", &path, e))?
            {
                let entry = entry.map_err(|e| VaultError::io("list vault move", &path, e))?;
                pending.push(relative.join(entry.file_name()));
            }
        } else if meta.is_file() {
            let mut file =
                File::open(&path).map_err(|e| VaultError::io("verify vault move", &path, e))?;
            let mut hash = blake3::Hasher::new();
            hash.update_reader(&mut file)
                .map_err(|e| VaultError::io("verify vault move", &path, e))?;
            manifest.insert(relative, Some(hash.finalize().to_hex().to_string()));
        } else {
            return Err(VaultError::Corrupt(
                "vault move contains a special file".to_owned(),
            ));
        }
    }
    Ok(manifest)
}

/// Held through relocation and selection publication; OS locks disappear after a crash.
pub(crate) struct Migration {
    _lock: File,
    journal: PathBuf,
    legacy: PathBuf,
    destination: PathBuf,
    moved: bool,
}

impl Migration {
    pub(crate) fn relocated(&self) -> bool {
        self.moved
    }

    pub(crate) fn begin(legacy: &Path, destination: &Path) -> Result<Self, VaultError> {
        check_path(legacy)?;
        check_path(destination)?;
        let container = destination
            .parent()
            .ok_or_else(|| VaultError::Corrupt("private vault has no container".to_owned()))?;
        create_private_dir_all(container)?;
        let lock_path = container.join("migration.lock");
        check_path(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(&lock_path)
            .map_err(|e| VaultError::io("open vault migration lock", &lock_path, e))?;
        lock.lock()
            .map_err(|e| VaultError::io("lock vault migration", &lock_path, e))?;
        let journal = container.join("migration.json");
        check_path(&journal)?;
        Ok(Self {
            _lock: lock,
            journal,
            legacy: legacy.to_path_buf(),
            destination: destination.to_path_buf(),
            moved: false,
        })
    }

    pub(crate) fn relocate(&mut self) -> Result<(), VaultError> {
        let existing = match std::fs::read(&self.journal) {
            Ok(raw) => Some(serde_json::from_slice::<Manifest>(&raw).map_err(|_| {
                VaultError::Corrupt(
                    "vault move journal is invalid; both folders were kept".to_owned(),
                )
            })?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(VaultError::io("read vault move journal", &self.journal, e)),
        };
        let old_exists = self
            .legacy
            .try_exists()
            .map_err(|e| VaultError::io("find old vault", &self.legacy, e))?;
        let new_exists = self
            .destination
            .try_exists()
            .map_err(|e| VaultError::io("find private vault", &self.destination, e))?;
        if old_exists && new_exists {
            return Err(VaultError::Corrupt(
                "both private vault locations exist; neither was changed".to_owned(),
            ));
        }
        if !old_exists && existing.is_none() {
            return Ok(());
        }
        let manifest = if let Some(manifest) = existing {
            manifest
        } else {
            let manifest = snapshot(&self.legacy)?;
            let raw =
                serde_json::to_vec(&manifest).map_err(|e| VaultError::Corrupt(e.to_string()))?;
            // A torn journal must never strand an intact source vault after a power loss.
            let temporary = self.journal.with_file_name(format!(
                ".migration-{}.tmp",
                hex::encode(vault_core::random_bytes::<8>())
            ));
            let publish = write_synced(&temporary, &raw).and_then(|()| {
                std::fs::hard_link(&temporary, &self.journal)
                    .map_err(|e| VaultError::io("publish vault move journal", &self.journal, e))
            });
            let _ = std::fs::remove_file(&temporary);
            publish?;
            sync_parent(&self.journal);
            manifest
        };
        if old_exists {
            if snapshot(&self.legacy)? != manifest {
                return Err(VaultError::Corrupt(
                    "old vault changed during migration; it was kept".to_owned(),
                ));
            }
            std::fs::rename(&self.legacy, &self.destination).map_err(|e| {
                VaultError::io("move private vault (old folder kept)", &self.legacy, e)
            })?;
            sync_parent(&self.legacy);
            sync_parent(&self.destination);
        }
        self.moved = true;
        let verified = snapshot(&self.destination).and_then(|actual| {
            if actual == manifest {
                Ok(())
            } else {
                Err(VaultError::Corrupt(
                    "private vault move did not verify".to_owned(),
                ))
            }
        });
        if let Err(error) = verified {
            self.rollback()?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn rollback(&mut self) -> Result<(), VaultError> {
        if self.moved {
            if self
                .legacy
                .try_exists()
                .map_err(|e| VaultError::io("check rollback folder", &self.legacy, e))?
            {
                return Err(VaultError::Corrupt(
                    "vault rollback blocked; both folders were kept".to_owned(),
                ));
            }
            std::fs::rename(&self.destination, &self.legacy).map_err(|e| {
                VaultError::io(
                    "vault rollback failed; data kept in private folder",
                    &self.destination,
                    e,
                )
            })?;
            sync_parent(&self.legacy);
            self.moved = false;
        }
        Ok(())
    }

    pub(crate) fn finish(&self) -> Result<(), VaultError> {
        match std::fs::remove_file(&self.journal) {
            Ok(()) => {
                sync_parent(&self.journal);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(VaultError::io("finish vault move", &self.journal, e)),
        }
    }
}

fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent()
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}
