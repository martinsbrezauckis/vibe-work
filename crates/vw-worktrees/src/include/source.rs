//! Reading the main checkout safely: links are recognised (never followed), a source file is checked on every
//! level of its path, `.worktreeinclude` is read as data, and git is asked which untracked files are ignored.
//!
//! Depends on: vw-changes (`git::Call`), `names`, `pattern`. Called by: `plan`, `copy`.
//! Invariants:
//! - Every level is looked at with `symlink_metadata`; a link at any level (symlink, junction, any reparse point)
//!   is refused, so no read leaves the checkout through a link that was there when we looked.
//! - A handle is compared with the `lstat` identity (device + inode on Unix; size + time elsewhere) after
//!   opening, so a file swapped between the look and the open is noticed.
//! - Git runs only through the hardened runner (no repository-defined program, no network, no hooks).
//!   Residual: a same-user process that swaps a directory for a link between our look and our open can still win
//!   that race; it already runs as the user (the same residual as every path check in this app).

use std::fs::Metadata;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use vw_changes::git::{Call, Index};

use super::pattern::Patterns;
use super::{INCLUDE_FILE, MAX_INCLUDE_FILE, MAX_PATTERNS};

const READ: Duration = Duration::from_secs(20);

/// True for a symlink, a junction or any other reparse point.
#[must_use]
pub(crate) fn is_link_like(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT: junctions, mount points, OneDrive online-only placeholders.
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// What reading `.worktreeinclude` found.
pub(crate) enum IncludeFileState {
    /// No such file.
    Missing,
    /// It exists but is not used (reason in plain words).
    Unusable(String),
    /// The patterns.
    Patterns(String),
}

/// Reads `<root>/.worktreeinclude` as data: a regular file (never a link), at most 64 KiB, valid text without
/// NUL, at most 500 patterns.
pub(crate) fn read_include_file(root: &Path) -> IncludeFileState {
    let path = root.join(INCLUDE_FILE);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return IncludeFileState::Missing;
    };
    let unusable =
        |why: &str| IncludeFileState::Unusable(format!("{INCLUDE_FILE} is not used: {why}."));
    if is_link_like(&meta) {
        return unusable("it is a link");
    }
    if !meta.is_file() {
        return unusable("it is not a regular file");
    }
    if meta.len() > MAX_INCLUDE_FILE {
        return unusable("it is larger than 64 KiB");
    }
    let Ok((file, _)) = super::safefs::open_source(root, &[INCLUDE_FILE]) else {
        return unusable("it could not be read safely");
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_INCLUDE_FILE + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_INCLUDE_FILE
    {
        return unusable("it could not be read");
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return unusable("it is not text");
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
    if text.contains('\0') {
        return unusable("it is not text");
    }
    if Patterns::parse(&text, false).len() > MAX_PATTERNS {
        return unusable("it holds more than 500 patterns");
    }
    IncludeFileState::Patterns(text)
}

/// git's `core.ignorecase` for the checkout (case-insensitive matching on Windows and macOS defaults).
pub(crate) fn ignore_case(root: &Path) -> bool {
    Call::new(root, Index::User)
        .args(["config", "--type=bool", "--get", "core.ignorecase"])
        .timeout(READ)
        .output()
        .is_ok_and(|o| o.ok() && String::from_utf8_lossy(&o.stdout).trim() == "true")
}

/// One entry of git's ignored listing.
pub(crate) enum Ignored {
    /// An ignored file (tracked files never appear).
    File(String),
    /// A wholly ignored directory, collapsed to one entry (no trailing slash).
    Dir(String),
}

/// `git ls-files --others --ignored --exclude-standard --directory -z` in the checkout: what git itself says
/// is ignored. Wholly ignored directories are not expanded. Entries that are not valid text are returned as
/// lossy text so the caller can list them as skipped (`Err` side of the pair).
///
/// # Errors
/// A message when git cannot list (for the report's `stopped`).
pub(crate) fn list_ignored(root: &Path) -> Result<(Vec<Ignored>, Vec<String>), String> {
    let out = Call::new(root, Index::User)
        .args([
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ])
        .timeout(READ)
        .output()
        .map_err(|e| format!("git could not list the ignored files: {e}"))?;
    if !out.ok() {
        return Err(format!(
            "git could not list the ignored files: {}",
            out.stderr.trim()
        ));
    }
    let mut list = Vec::new();
    let mut bad = Vec::new();
    for raw in out.stdout.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        match std::str::from_utf8(raw) {
            Ok(text) => match text.strip_suffix('/') {
                Some(dir) => list.push(Ignored::Dir(dir.to_owned())),
                None => list.push(Ignored::File(text.to_owned())),
            },
            Err(_) => bad.push(String::from_utf8_lossy(raw).into_owned()),
        }
    }
    Ok((list, bad))
}
