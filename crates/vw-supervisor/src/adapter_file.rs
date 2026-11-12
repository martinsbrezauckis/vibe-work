//! Delivers the compiled-in Vibe guards through a private, content-addressed runtime file.
//! Uses vw-platform's owner-only files and SHA-256; the supervisor prepares it before every spawn.
//! No temp-directory fallback. Python verifies the same bytes it executes, closing the check/open race.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::{CHILD_STOP_ADAPTER, SupervisorError};

/// Fixed-size isolated loader, independent of the guard source's size. Its digest check covers
/// the exact in-memory bytes passed to `compile()`, including after a file replacement during spawn.
pub const ADAPTER_BOOTSTRAP: &str = include_str!("adapter_bootstrap.py");

/// SHA-256 of the adapter embedded in this build, never read from a mutable file or its name.
#[must_use]
pub fn adapter_digest() -> &'static str {
    static DIGEST: OnceLock<String> = OnceLock::new();
    DIGEST.get_or_init(|| format!("{:x}", Sha256::digest(CHILD_STOP_ADAPTER.as_bytes())))
}

/// The app-owned adapter path for this build. Distinct builds coexist; no live file is overwritten.
/// The supplied Vibe home belongs to the app (the supervisor refuses a relative or linked home).
#[must_use]
pub fn adapter_path(home: &Path) -> PathBuf {
    home.join(".launcher")
        .join(format!("{}.py", adapter_digest()))
}

pub(crate) fn prepare(home: &Path) -> Result<PathBuf, SupervisorError> {
    let path = adapter_path(home);
    prepare_file(home, &path).map_err(|source| SupervisorError::AdapterFile {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn plain(path: &Path, directory: bool) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    let linked = meta.file_type().is_symlink();
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        linked || meta.file_attributes() & 0x400 != 0 // Every reparse point, including junctions.
    };
    if linked || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err(io::Error::other(
            "launch adapter path is not a plain file or directory",
        ));
    }
    Ok(())
}

fn private_dir(path: &Path) -> io::Result<()> {
    match plain(path, true) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        result => result?,
    }
    vw_platform::ensure_private_dir(path).map_err(io::Error::other)?;
    plain(path, true)
}

fn prepare_file(home: &Path, path: &Path) -> io::Result<()> {
    if !home.is_absolute() {
        return Err(io::Error::other(
            "launch adapter requires an absolute app-owned Vibe home",
        ));
    }
    private_dir(home)?;
    private_dir(&home.join(".launcher"))?;
    // Publish a complete file from a CREATE_NEW owner-only sibling, then rename. Concurrent first
    // starts publish identical bytes, never a partial file. Existing content is checked, not repaired.
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            vw_platform::replace_private(path, CHILD_STOP_ADAPTER.as_bytes())
                .map_err(io::Error::other)?;
        }
        Err(error) => return Err(error),
    }
    plain(path, false)?;
    if !vw_platform::is_user_only(path).map_err(io::Error::other)? {
        return Err(io::Error::other("launch adapter is not owner-only"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(CHILD_STOP_ADAPTER.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    if format!("{:x}", Sha256::digest(&bytes)) != adapter_digest() {
        return Err(io::Error::other("launch adapter integrity check failed"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "adapter_file_tests.rs"]
mod tests;
