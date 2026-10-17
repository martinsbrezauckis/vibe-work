//! Temporary folders for proposal fixtures; std and tempfile only.
//! Proposals refuse a protected folder reached through a symbolic link, and the macOS system temporary
//! folder lies below the `/var` link, so fixtures are created below the resolved temporary folder.

#![allow(dead_code, clippy::unwrap_used)]

use std::path::PathBuf;

use tempfile::TempDir;

/// A new empty folder whose path has no symbolic-link component.
pub fn tempdir() -> TempDir {
    TempDir::new_in(base()).unwrap()
}

/// The system temporary folder, resolved on Unix. Windows keeps the folder as given: its temporary folder is
/// not a link, and resolving it would only add the `\\?\` prefix.
fn base() -> PathBuf {
    let base = std::env::temp_dir();
    if cfg!(unix) {
        base.canonicalize().unwrap()
    } else {
        base
    }
}
