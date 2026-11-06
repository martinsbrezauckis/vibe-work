//! Ordinary atomic file replacement for core editors and exports.
//! Uses exclusive random sibling files and explicit permission policy; capability-relative and private-ACL
//! writers remain separate. Callers validate their destination and retain stale-content checks.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;

/// Access metadata to give an ordinary replacement file. Neither option preserves timestamps or ownership.
#[derive(Debug, Clone, Copy)]
pub enum MetadataPolicy {
    /// Use the directory's defaults, as for a new export; replace the final name even if it is a link.
    NewFile,
    /// Retain a regular file's permissions (Unix mode; Windows read-only flag and DACL/protection).
    /// A missing destination uses directory defaults; links and other non-files are refused.
    PreservePermissions,
}

/// Writes a whole file through a new sibling and an atomic rename over `target`.
/// The file is synced before publication. Random name collisions are retried without touching the existing
/// entry. Failures remove the owned temporary file. Does not create parents or sync the parent directory.
///
/// # Errors
/// An I/O error from metadata, creation, writing, permissions, sync or replacement.
pub fn replace_file(target: &Path, bytes: &[u8], metadata: MetadataPolicy) -> io::Result<()> {
    replace_with(target, metadata, random_suffix, |file| {
        file.write_all(bytes)
    })
}

fn random_suffix() -> io::Result<u64> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes)
        .map_err(|error| io::Error::other(format!("no random name: {error}")))?;
    Ok(u64::from_le_bytes(bytes))
}

fn original_file(target: &Path, policy: MetadataPolicy) -> io::Result<Option<File>> {
    if matches!(policy, MetadataPolicy::NewFile) {
        return Ok(None);
    }
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.is_file() => File::open(target).map(Some),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn create_temp(
    target: &Path,
    mut suffix: impl FnMut() -> io::Result<u64>,
) -> io::Result<(PathBuf, File)> {
    let folder = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no folder"))?;
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    for _ in 0..8 {
        let mut leaf = std::ffi::OsString::from(".");
        leaf.push(name);
        leaf.push(format!(".vw-{:016x}.tmp", suffix()?));
        let temp = folder.join(leaf);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        // The owned temp handle must be able to receive the original DACL before it holds any content.
        #[cfg(windows)]
        windows::write_access(&mut options);
        match options.open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary name",
    ))
}

fn replace_with(
    target: &Path,
    metadata: MetadataPolicy,
    suffix: impl FnMut() -> io::Result<u64>,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let original = original_file(target, metadata)?;
    let (temp, mut file) = create_temp(target, suffix)?;
    let result = (|| {
        if let Some(original) = &original {
            #[cfg(windows)]
            windows::copy_dacl(original, &file)?;
            file.set_permissions(original.metadata()?.permissions())?;
        }
        write(&mut file)?;
        file.sync_all()?;
        fs::rename(&temp, target)
    })();
    if result.is_err() {
        // Read-only is an attribute on Windows; clear it only on our owned handle for cleanup.
        #[cfg(windows)]
        windows::allow_cleanup(&file);
    }
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests;
