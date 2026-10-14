//! "Open with default app" and "Show in folder" for one file. Never runs a file: only kinds on a
//! positive list of documents, pictures, media and archives are handed to the OS opener; HTML and SVG are "active"
//! (the default browser runs their scripts with network access) and the caller must have asked the user first;
//! everything else, and any file whose first bytes are a program (ELF, PE, Mach-O, `#!` script) or that carries an
//! execute permission bit, is refused whatever its name says.
//!
//! Depends on: std only (`xdg-open`, `open`, `explorer` with a fixed program name and the canonical path as one
//! argument, never through a shell), `reveal` (the Windows local-drive rule). Called by: vw-core-api (`files open`,
//! `files reveal`) after it checked that the file lies inside an allowed folder.
//! Invariants:
//! - The class is decided again here on the CANONICAL path and the opened file's first bytes, right before the
//!   opener starts (the core's earlier check does not count).
//! - Windows: network and device paths are refused before any filesystem access (same rule as `reveal`).
//! - The child is detached and reaped on a helper thread (no zombie, no waiting).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::reveal::windows_local_drive_path;

/// How a file may be handed to the OS opener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenClass {
    /// A document, picture, media file or archive: opened directly when the user asks.
    Passive,
    /// HTML or SVG: the default app (a browser) runs its scripts with network access; ask the user first.
    Active,
    /// Anything else, or a program: never opened.
    Refused,
}

/// Extensions opened directly. Legacy Office formats (`.doc`, `.xls`, `.ppt`) and macro-enabled ones are left
/// out: they can carry macros.
const PASSIVE: &[&str] = &[
    "pdf", "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "avif", "heic", "ico", "txt",
    "md", "markdown", "log", "csv", "tsv", "json", "yaml", "yml", "toml", "rtf", "docx", "xlsx",
    "pptx", "odt", "ods", "odp", "epub", "mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "mp4",
    "m4v", "mov", "webm", "mkv", "avi", "zip", "tar", "gz", "tgz", "7z",
];
/// Extensions opened only after the user confirmed (a browser runs them).
const ACTIVE: &[&str] = &["html", "htm", "xhtml", "svg"];

/// True when `head` (a file's first bytes) starts like a program.
#[must_use]
pub fn looks_like_program(head: &[u8]) -> bool {
    const MAGIC: &[&[u8]] = &[
        b"\x7fELF",
        b"MZ",
        b"#!",
        b"\xfe\xed\xfa\xce",
        b"\xfe\xed\xfa\xcf",
        b"\xce\xfa\xed\xfe",
        b"\xcf\xfa\xed\xfe",
        b"\xca\xfe\xba\xbe",
        // Linux launcher (`.desktop`): `xdg-open` can choose the handler from the content, whatever the name says.
        b"[Desktop Entry]",
    ];
    MAGIC.iter().any(|m| head.starts_with(m))
}

/// The class of a file named `name` whose first bytes are `head` (pure: tested on every platform).
#[must_use]
pub fn open_class(name: &str, head: &[u8]) -> OpenClass {
    if looks_like_program(head) {
        return OpenClass::Refused;
    }
    let Some((_, ext)) = name.rsplit_once('.') else {
        return OpenClass::Refused;
    };
    let ext = ext.to_ascii_lowercase();
    if PASSIVE.contains(&ext.as_str()) {
        OpenClass::Passive
    } else if ACTIVE.contains(&ext.as_str()) {
        OpenClass::Active
    } else {
        OpenClass::Refused
    }
}

/// Why a file was not opened or shown.
#[derive(Debug, thiserror::Error)]
pub enum OpenFileError {
    /// Not an absolute path to an existing regular file (after resolving links).
    #[error("not an existing file: {0}")]
    NotAFile(String),
    /// A network or device path (Windows): never opened.
    #[error("not a file on a local drive: {0}")]
    NotLocal(String),
    /// The file could run a program, or its kind is not on the list of files Vibe Work opens.
    #[error(
        "Vibe Work does not open this kind of file with another app (it could run a program). Use Show in folder."
    )]
    NotOpenable,
    /// HTML or SVG without the user's confirmation.
    #[error("opening this file runs its scripts in your browser; it needs your confirmation")]
    NeedsConfirmation,
    /// The opener program could not be started.
    #[error("the default app could not be started: {0}")]
    Spawn(String),
}

/// The canonical regular file for `path` (absolute, local on Windows).
fn canonical_file(path: &Path) -> Result<PathBuf, OpenFileError> {
    let shown = || path.display().to_string();
    if !path.is_absolute() {
        return Err(OpenFileError::NotAFile(shown()));
    }
    if cfg!(windows) && windows_local_drive_path(&path.to_string_lossy()).is_none() {
        return Err(OpenFileError::NotLocal(shown()));
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| OpenFileError::NotAFile(shown()))?;
    let canonical = if cfg!(windows) {
        let text = canonical.to_string_lossy().into_owned();
        PathBuf::from(windows_local_drive_path(&text).ok_or(OpenFileError::NotLocal(text))?)
    } else {
        canonical
    };
    if !canonical.is_file() {
        return Err(OpenFileError::NotAFile(shown()));
    }
    Ok(canonical)
}

/// The class of the file at `path`, judged on its canonical path, its first 512 bytes and (Unix) its permission
/// bits: a file anyone may execute is [`OpenClass::Refused`].
///
/// # Errors
/// [`OpenFileError::NotAFile`] / [`OpenFileError::NotLocal`].
pub fn file_open_class(path: &Path) -> Result<(PathBuf, OpenClass), OpenFileError> {
    let canonical = canonical_file(path)?;
    let mut head = [0_u8; 512];
    let mut file = std::fs::File::open(&canonical)
        .map_err(|_| OpenFileError::NotAFile(path.display().to_string()))?;
    let read = file.read(&mut head).unwrap_or(0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Fail closed: unknown permissions count as executable.
        let mode = file.metadata().map_or(0o111, |m| m.permissions().mode());
        if mode & 0o111 != 0 {
            return Ok((canonical, OpenClass::Refused));
        }
    }
    let name = canonical
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let class = open_class(&name, head.get(..read).unwrap_or_default());
    Ok((canonical, class))
}

fn spawn_detached(program: &str, args: Vec<std::ffi::OsString>) -> Result<(), OpenFileError> {
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|error| OpenFileError::Spawn(error.to_string()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens `path` with the user's default app. `confirmed`: the user agreed in a dialog (needed for
/// [`OpenClass::Active`] files).
///
/// # Errors
/// Every [`OpenFileError`] for the reasons named in the module docs.
pub fn open_file(path: &Path, confirmed: bool) -> Result<(), OpenFileError> {
    let (canonical, class) = file_open_class(path)?;
    match class {
        OpenClass::Refused => return Err(OpenFileError::NotOpenable),
        OpenClass::Active if !confirmed => return Err(OpenFileError::NeedsConfirmation),
        OpenClass::Active | OpenClass::Passive => {}
    }
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    spawn_detached(program, vec![canonical.into_os_string()])
}

/// Shows `path` in the file manager: selected (Windows, macOS) or its folder opened (Linux desktops).
///
/// # Errors
/// [`OpenFileError::NotAFile`] / [`OpenFileError::NotLocal`] / [`OpenFileError::Spawn`].
pub fn reveal_file(path: &Path) -> Result<(), OpenFileError> {
    let canonical = canonical_file(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer parses `/select,"<path>"` itself; Windows paths cannot contain `"`.
        let mut child = Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", canonical.display()))
            .spawn()
            .map_err(|error| OpenFileError::Spawn(error.to_string()))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if cfg!(target_os = "macos") {
            spawn_detached("open", vec!["-R".into(), canonical.into_os_string()])
        } else {
            let folder = canonical
                .parent()
                .map_or_else(|| canonical.clone(), Path::to_path_buf);
            spawn_detached("xdg-open", vec![folder.into_os_string()])
        }
    }
}
