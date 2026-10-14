//! Writing and deleting working-tree files for a revert, done by us: git never
//! writes the working tree here (no `restore`, `checkout` or `apply`, which run smudge filters).
//!
//! Depends on: `paths`, `error`, std. Called by: `review`.
//! Invariants:
//! - The path is a plain repository-relative path (`paths::inside`), and no folder between the root and the
//!   file is a symlink (a symlinked parent could point outside the working tree).
//! - A file is replaced atomically: written next to it under a temporary name, then renamed over it, so a
//!   crash never leaves half a file. A folder is never deleted or replaced.

use std::path::Path;

use crate::error::ChangesError;
use crate::paths;

fn checked(root: &Path, rel: &[u8]) -> Result<std::path::PathBuf, ChangesError> {
    let full = paths::inside_bytes(root, rel)?;
    if paths::has_symlinked_parent(root, &full) {
        return Err(ChangesError::Refused(format!(
            "{} is inside a symlinked folder: not changed",
            paths::display(rel)
        )));
    }
    Ok(full)
}

/// Writes `bytes` as `rel` with git `mode` (`100644`, `100755`, `120000` = symlink to `bytes`).
///
/// # Errors
/// `Refused` for unsafe paths or a folder in the way; `Io` on write failures.
pub(crate) fn write(root: &Path, rel: &[u8], bytes: &[u8], mode: &str) -> Result<(), ChangesError> {
    let shown = paths::display(rel);
    let full = checked(root, rel)?;
    if let Ok(meta) = std::fs::symlink_metadata(&full)
        && meta.is_dir()
    {
        return Err(ChangesError::Refused(format!(
            "{shown} is a folder now: not replaced"
        )));
    }
    let parent = full
        .parent()
        .ok_or_else(|| ChangesError::Refused(format!("{shown} has no parent folder")))?;
    std::fs::create_dir_all(parent).map_err(|e| ChangesError::io("create", parent, e))?;
    // Created folders could have raced into symlinks; check again before writing into them.
    if paths::has_symlinked_parent(root, &full) {
        return Err(ChangesError::Refused(format!(
            "{shown} is inside a symlinked folder: not changed"
        )));
    }
    // A fixed temporary name (not derived from the file name, which may not be text).
    let tmp = parent.join(format!(".vw-revert-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    if mode == "120000" {
        make_symlink(bytes, &tmp)?;
    } else {
        write_new(&tmp, bytes, mode == "100755")?;
    }
    std::fs::rename(&tmp, &full).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ChangesError::io("replace", &full, e)
    })
}

fn write_new(path: &Path, bytes: &[u8], executable: bool) -> Result<(), ChangesError> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| ChangesError::io("create", path, e))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| ChangesError::io("write", path, e))?;
    set_executable(path, executable)
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<(), ChangesError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| ChangesError::io("set permissions on", path, e))
}

/// Windows has no executable bit (git keeps it in the index only); the signature matches the Unix one.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), ChangesError> {
    Ok(())
}

#[cfg(unix)]
fn make_symlink(target: &[u8], path: &Path) -> Result<(), ChangesError> {
    use std::os::unix::ffi::OsStrExt;
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), path)
        .map_err(|e| ChangesError::io("create symlink", path, e))
}

/// Without symlink rights (Windows), git writes the link target as a plain file; so do we.
#[cfg(not(unix))]
fn make_symlink(target: &[u8], path: &Path) -> Result<(), ChangesError> {
    write_new(path, target, false)
}

/// Deletes the file (or symlink) `rel`; a missing file is fine, a folder is refused.
///
/// # Errors
/// `Refused` for unsafe paths or a folder; `Io` on failures.
pub(crate) fn delete(root: &Path, rel: &[u8]) -> Result<(), ChangesError> {
    let shown = paths::display(rel);
    let full = checked(root, rel)?;
    match std::fs::symlink_metadata(&full) {
        Ok(meta) if meta.is_dir() => Err(ChangesError::Refused(format!(
            "{shown} is a folder: not deleted"
        ))),
        Ok(_) => std::fs::remove_file(&full).map_err(|e| ChangesError::io("delete", &full, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ChangesError::io("inspect", &full, e)),
    }
}

/// Renames the file `from` to `to` in the same folder (a case-only rename put back on a case-insensitive
/// file system, where the two names are one file).
///
/// # Errors
/// `Refused` for unsafe paths or different folders; `Io` on failures.
pub(crate) fn rename_back(root: &Path, from: &[u8], to: &[u8]) -> Result<(), ChangesError> {
    let source = checked(root, from)?;
    let target = checked(root, to)?;
    if source.parent() != target.parent() {
        return Err(ChangesError::Refused(format!(
            "{} and {} are in different folders: not renamed",
            paths::display(from),
            paths::display(to)
        )));
    }
    std::fs::rename(&source, &target).map_err(|e| ChangesError::io("rename", &source, e))
}
