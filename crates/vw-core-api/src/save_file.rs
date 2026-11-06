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
        vw_platform::atomic_file::replace_file(
            &target,
            &bytes,
            vw_platform::atomic_file::MetadataPolicy::NewFile,
        )
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
