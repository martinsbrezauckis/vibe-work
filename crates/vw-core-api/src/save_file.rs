//! Writes a file the user chose in a save dialog (or the Debug API named, acting as the user): "Share a page as one
//! file" and "Export as Markdown".
//!
//! Depends on: `pages` (`blocking`), `error`. Called by: `pages_share`, `session_export`.
//! Invariants: the destination is absolute, its folder exists, it is not a folder; the file is written whole (a
//! temporary file next to it, renamed over it), so a reader never sees half a file; a failed write leaves no
//! temporary file behind.
//! - **The temporary file is created new** (`create_new`: `O_CREAT|O_EXCL` / `CREATE_NEW`, which never follows a
//!   symbolic link) under a random name, so a folder from outside (a cloned repository) cannot plant a link at the
//!   temporary name and have the bytes written through it elsewhere (the earlier fixed name
//!   `<name>.vw-export-tmp` was followed). The rename replaces the final name itself and never follows a link there.
//! - A destination the user chose in the native save dialog is the user's own choice, also when it is a link: the
//!   rename replaces that name in its folder.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, ErrorCode};
use crate::pages::blocking;

/// Writes `bytes` to `destination` and returns the path written (for the answer).
///
/// # Errors
/// `invalidParams` for a relative path, a missing folder or a folder as the target; `internal` when the write fails.
pub(crate) async fn write_whole(bytes: Vec<u8>, destination: &Path) -> Result<String, CoreError> {
    let bad = |why: &str| {
        CoreError::new(
            ErrorCode::InvalidParams,
            format!("cannot save there: {why}"),
        )
    };
    if !destination.is_absolute() {
        return Err(bad("the path must be absolute"));
    }
    let folder = destination.parent().ok_or_else(|| bad("no folder"))?;
    if !folder.is_dir() {
        return Err(bad("the folder does not exist"));
    }
    if destination.is_dir() {
        return Err(bad("that is a folder"));
    }
    let target: PathBuf = destination.to_path_buf();
    blocking(move || {
        replace_file(&target, &bytes, None)
            .map_err(|e| {
                CoreError::new(
                    ErrorCode::Internal,
                    format!("the file could not be written: {e}"),
                )
            })
            .map(|()| target.display().to_string())
    })
    .await?
}

/// Replaces `target` with `bytes` through a new temporary file in the same folder (random name, created with
/// `create_new`, so never through a planted link), with `permissions` when given, then renamed over `target`.
/// Blocking. The temporary file is removed on any failure.
///
/// # Errors
/// The I/O error of the create, write, sync, permission change or rename.
pub(crate) fn replace_file(
    target: &Path,
    bytes: &[u8],
    permissions: Option<&std::fs::Permissions>,
) -> std::io::Result<()> {
    use std::io::Write as _;
    let folder = target
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no folder"))?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    for _ in 0..8 {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random)
            .map_err(|e| std::io::Error::other(format!("no random name: {e}")))?;
        let suffix = u64::from_le_bytes(random);
        let temp = folder.join(format!(".{name}.vw-{suffix:016x}.tmp"));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let written = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| match permissions {
                Some(p) => std::fs::set_permissions(&temp, p.clone()),
                None => Ok(()),
            });
        drop(file);
        let done = written.and_then(|()| std::fs::rename(&temp, target));
        if done.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        return done;
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free temporary name",
    ))
}
