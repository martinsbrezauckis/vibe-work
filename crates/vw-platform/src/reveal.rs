//! Show a folder in the operating system's file manager.
//!
//! Depends on: std only (`explorer`, `open -R`, `xdg-open` with a fixed program name and the path as
//! one argument, never through a shell). Called by: vw-host (`Host::reveal_folder`) after the host
//! has checked that a session or saved project's main folder uses that path.
//! Invariants:
//! - Only an existing directory is opened, judged on its CANONICAL path (symlinks and junctions
//!   resolved), and the canonical path is what the opener gets: a link to a file or to a program
//!   cannot be passed off as a folder.
//! - macOS uses `open -R` (reveal in Finder, the item selected): a `.app` bundle is a directory, and
//!   plain `open` would launch it.
//! - Windows refuses every path that is not on a local drive (`\\server\share`, `//server/share`,
//!   `\\?\UNC\...`, `\\.\...` device paths, `\\?\GLOBALROOT\...`): opening a network path makes
//!   Windows authenticate to that server with the user's credentials. The check runs on the given
//!   text before any filesystem access, and again on the canonical path (a local junction can point
//!   to a share). Residual: resolving such a junction already touches the share; refusing it
//!   afterwards still keeps Explorer from opening it.
//! - The child is detached and reaped on a helper thread (no zombie, no waiting).
//! - Explorer reports exit code 1 even on success, so the exit status is not inspected.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Why a folder could not be shown.
#[derive(Debug, thiserror::Error)]
pub enum RevealError {
    /// Not an absolute path to an existing directory (after resolving links).
    #[error("not an existing folder: {0}")]
    NotAFolder(String),
    /// A network or device path (Windows): never opened.
    #[error("not a folder on a local drive: {0}")]
    NotLocal(String),
    /// The file manager could not be started.
    #[error("the file manager could not be started: {0}")]
    Spawn(String),
}

/// The file manager opener of one operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opener {
    /// Windows Explorer.
    Explorer,
    /// macOS Finder through `open -R`.
    Finder,
    /// Linux and other Unix desktops (`xdg-open`).
    XdgOpen,
}

impl Opener {
    /// The opener of the platform this binary was built for.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Explorer
        } else if cfg!(target_os = "macos") {
            Self::Finder
        } else {
            Self::XdgOpen
        }
    }

    /// Program and arguments that show `path` (already checked by [`reveal_target`]).
    #[must_use]
    pub fn command(self, path: &Path) -> (&'static str, Vec<OsString>) {
        match self {
            Self::Explorer => ("explorer", vec![path.as_os_str().to_owned()]),
            Self::Finder => ("open", vec!["-R".into(), path.as_os_str().to_owned()]),
            Self::XdgOpen => ("xdg-open", vec![path.as_os_str().to_owned()]),
        }
    }
}

/// Windows: `text` as Explorer should get it when it names a local drive (`C:\x`, or the verbatim
/// `\\?\C:\x` that `canonicalize` returns, prefix dropped); `None` for every other form (see the
/// module invariants). Pure string logic, so it is tested on every platform.
#[must_use]
pub fn windows_local_drive_path(text: &str) -> Option<String> {
    let drive = |s: &str| {
        let b = s.as_bytes();
        b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'\\' || b[2] == b'/')
    };
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return drive(rest).then(|| rest.to_owned());
    }
    drive(text).then(|| text.to_owned())
}

/// The path to hand to the opener: absolute, local (Windows), existing, a directory, canonical.
///
/// # Errors
/// [`RevealError::NotAFolder`] for a relative, missing or non-directory path (also when a link
/// resolves to a file); [`RevealError::NotLocal`] for a network or device path on Windows.
pub fn reveal_target(path: &Path) -> Result<PathBuf, RevealError> {
    let shown = || path.display().to_string();
    if !path.is_absolute() {
        return Err(RevealError::NotAFolder(shown()));
    }
    // Before any filesystem access: even a metadata read of a UNC path contacts the server.
    if cfg!(windows) && windows_local_drive_path(&path.to_string_lossy()).is_none() {
        return Err(RevealError::NotLocal(shown()));
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| RevealError::NotAFolder(shown()))?;
    let canonical = if cfg!(windows) {
        let text = canonical.to_string_lossy().into_owned();
        PathBuf::from(windows_local_drive_path(&text).ok_or(RevealError::NotLocal(text))?)
    } else {
        canonical
    };
    if !canonical.is_dir() {
        return Err(RevealError::NotAFolder(shown()));
    }
    Ok(canonical)
}

/// Opens `path` in the file manager (see [`reveal_target`] for what is accepted).
///
/// # Errors
/// [`RevealError::NotAFolder`] / [`RevealError::NotLocal`] from [`reveal_target`];
/// [`RevealError::Spawn`] when the opener program cannot start.
pub fn reveal_folder(path: &Path) -> Result<(), RevealError> {
    let target = reveal_target(path)?;
    let (program, args) = Opener::current().command(&target);
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|error| RevealError::Spawn(error.to_string()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
