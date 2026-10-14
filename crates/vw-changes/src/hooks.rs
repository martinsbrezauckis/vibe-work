//! The empty folder every git call of this crate uses as `core.hooksPath`: it lives only in the
//! app's private data root, made by [`crate::prepare_root`]; without a valid one, git calls refuse.
//!
//! Depends on: vw-platform (`ensure_private_dir`), `error`. Called by: `store::prepare_root` (registers),
//! `git` (every call asks for the folder).
//! Invariants:
//! - No fallback: before, a missing `prepare_root` used a folder in the shared temp folder, and kept using
//!   it when making it private failed, so a hook planted there could run.
//! - A folder is registered only when it is a plain directory (not a symlink or junction), private
//!   (`ensure_private_dir`: user-only, owned by us) and empty.
//! - Every call checks the folder again (plain directory, empty; on Unix also owner and mode as registered):
//!   a folder deleted and re-created by someone else, or given a file, is never used. Several data roots can
//!   be registered (test processes run many cores whose folders are deleted); the first valid one is used,
//!   missing ones are dropped.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::error::ChangesError;

/// A registered hooks folder and, on Unix, the owner it had when registered.
struct Registered {
    path: PathBuf,
    #[cfg(unix)]
    uid: u32,
}

static REGISTERED: Mutex<Vec<Registered>> = Mutex::new(Vec::new());

/// Makes `folder` (inside a private data root) the hooks folder after checking it.
///
/// # Errors
/// `Refused` when it is a symlink, not a directory, not private or not empty; `Io` when it cannot be made.
pub(crate) fn register(folder: &Path) -> Result<(), ChangesError> {
    let refuse = |why: &str| {
        ChangesError::Refused(format!(
            "the private hooks folder {} {why}",
            folder.display()
        ))
    };
    vw_platform::ensure_private_dir(folder).map_err(|e| refuse(&format!("is unusable: {e}")))?;
    let meta =
        std::fs::symlink_metadata(folder).map_err(|e| ChangesError::io("inspect", folder, e))?;
    if !usable(folder, &meta) {
        return Err(refuse("is not an empty plain folder"));
    }
    let entry = Registered {
        path: folder.to_path_buf(),
        #[cfg(unix)]
        uid: std::os::unix::fs::MetadataExt::uid(&meta),
    };
    let mut registered = REGISTERED.lock().unwrap_or_else(PoisonError::into_inner);
    registered.retain(|r| r.path != entry.path);
    registered.push(entry);
    Ok(())
}

/// The hooks folder for the next git call.
///
/// # Errors
/// `Refused` when no registered folder is still valid (never prepared, deleted, changed).
pub(crate) fn current() -> Result<PathBuf, ChangesError> {
    let mut registered = REGISTERED.lock().unwrap_or_else(PoisonError::into_inner);
    registered.retain(|r| r.path.exists());
    registered
        .iter()
        .find(|r| {
            std::fs::symlink_metadata(&r.path).is_ok_and(|meta| {
                #[cfg(unix)]
                let owner = std::os::unix::fs::MetadataExt::uid(&meta) == r.uid
                    && no_group_or_other(std::os::unix::fs::PermissionsExt::mode(
                        &meta.permissions(),
                    ));
                #[cfg(not(unix))]
                let owner = true;
                owner && usable(&r.path, &meta)
            })
        })
        .map(|r| r.path.clone())
        .ok_or_else(|| {
            ChangesError::Refused(
                "the Changes tab's private hooks folder is not ready: git is not run without it"
                    .into(),
            )
        })
}

/// A plain (not linked) directory with nothing in it.
fn usable(folder: &Path, meta: &std::fs::Metadata) -> bool {
    meta.is_dir()
        && !meta.file_type().is_symlink()
        && std::fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_none())
}

/// No group or other permission bits.
#[cfg(unix)]
#[allow(
    clippy::verbose_bit_mask,
    reason = "`mode & 0o077 == 0` reads as 'no group or other bits'"
)]
fn no_group_or_other(mode: u32) -> bool {
    mode & 0o077 == 0
}
