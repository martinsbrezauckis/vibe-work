//! Path handling between git, patches and the disk: git's C-style quoting (read and write), literal
//! pathspecs, and the check that a path from git or from a client names a file inside the working tree.
//!
//! Depends on: std, `error`. Called by: `diff`, `parse`, `review`, `baseline`.
//! Invariants:
//! - Paths are repository-relative, `/`-separated strings. A path with an empty, `.`, `..` or `.git`
//!   component, an absolute path or a drive prefix is refused before any disk access (the ShellX idea
//!   `safe_git_relative_path`, `session_git.rs:1097-1116` @ e38c2b5c).
//! - Pathspecs we pass to git are always `:(literal)`, so `*`, `?` or `[` in a file name never glob.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::error::ChangesError;

/// Undoes git's C-style quoting of a path (`"a\tb"`, `"\303\244"`); returns unquoted input unchanged.
#[must_use]
pub fn unquote(raw: &[u8]) -> Vec<u8> {
    if raw.len() < 2 || raw[0] != b'"' || raw[raw.len() - 1] != b'"' {
        return raw.to_vec();
    }
    let body = &raw[1..raw.len() - 1];
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        let byte = body[i];
        if byte != b'\\' || i + 1 >= body.len() {
            out.push(byte);
            i += 1;
            continue;
        }
        let next = body[i + 1];
        let simple = match next {
            b'a' => Some(0x07),
            b'b' => Some(0x08),
            b't' => Some(b'\t'),
            b'n' => Some(b'\n'),
            b'v' => Some(0x0b),
            b'f' => Some(0x0c),
            b'r' => Some(b'\r'),
            b'"' => Some(b'"'),
            b'\\' => Some(b'\\'),
            _ => None,
        };
        if let Some(value) = simple {
            out.push(value);
            i += 2;
            continue;
        }
        let digits = body
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)));
        if let Some(digits) = digits {
            let value = digits
                .iter()
                .fold(0_u32, |acc, d| acc * 8 + u32::from(d - b'0'));
            out.push(u8::try_from(value & 0xff).unwrap_or(0));
            i += 4;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    out
}

/// Quotes `path` the way git does when it must (control characters, `"`, `\`), so it can go into a
/// patch header; other paths are returned as they are.
#[must_use]
pub fn quote(path: &str) -> String {
    let needs = path
        .bytes()
        .any(|b| b < 0x20 || b == b'"' || b == b'\\' || b == 0x7f);
    if !needs {
        return path.to_owned();
    }
    // Non-ASCII characters stay as they are: git reads them back unescaped (core.quotePath=false).
    let mut out = String::from("\"");
    for ch in path.chars() {
        match ch {
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 || u32::from(c) == 0x7f => {
                let _ = write!(out, "\\{:03o}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `:(literal)<path>`: a pathspec that matches exactly this path.
#[must_use]
pub fn literal(path: &str) -> OsString {
    OsString::from(format!(":(literal){path}"))
}

/// `:(exclude,literal)<path>`.
#[must_use]
pub fn exclude_literal(path: &str) -> OsString {
    OsString::from(format!(":(exclude,literal){path}"))
}

/// Checks that `path` is a plain repository-relative path and returns it joined to `root`.
///
/// # Errors
/// `Refused` for empty, absolute, drive-prefixed paths and for `.`, `..`, `.git` components.
pub fn inside(root: &Path, path: &str) -> Result<PathBuf, ChangesError> {
    let refuse = |why: &str| ChangesError::Refused(format!("path {path:?} {why}"));
    if path.is_empty() {
        return Err(refuse("is empty"));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(refuse("is absolute"));
    }
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return Err(refuse("has a drive prefix"));
    }
    let mut joined = root.to_path_buf();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(refuse("has an empty, '.' or '..' component"));
        }
        if part.eq_ignore_ascii_case(".git") {
            return Err(refuse("points into .git"));
        }
        if cfg!(windows) && part.contains('\\') {
            return Err(refuse("contains a backslash"));
        }
        joined.push(part);
    }
    Ok(joined)
}

/// True when some directory between `root` (exclusive) and `path` (exclusive) is a symlink: deleting or
/// writing through it could land outside the working tree (the symlinked-parent class).
#[must_use]
pub fn has_symlinked_parent(root: &Path, path: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(root) else {
        return true;
    };
    let mut current = root.to_path_buf();
    let mut parts = rest.components().peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            break;
        }
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            _ => {}
        }
    }
    false
}

/// How a git path (bytes) is shown and named in the API: valid UTF-8 as is, every byte that is not part of
/// valid UTF-8 as `\xNN`, and a literal backslash as `\\` (so the mapping is one-to-one: two different file
/// names never share a shown name). integrity check: no name may fall out of the list.
#[must_use]
pub fn display(raw: &[u8]) -> String {
    let mut out = String::new();
    for chunk in raw.utf8_chunks() {
        for ch in chunk.valid().chars() {
            if ch == '\\' {
                out.push_str("\\\\");
            } else {
                out.push(ch);
            }
        }
        for byte in chunk.invalid() {
            let _ = write!(out, "\\x{byte:02x}");
        }
    }
    out
}

/// [`inside`] for a git path given as bytes (the form `-z` output and index files hold).
///
/// # Errors
/// `Refused` for empty, absolute or drive-prefixed paths, `.`/`..`/`.git` components, and (Windows) names
/// that are not UTF-8 (git for Windows only writes UTF-8 paths).
pub fn inside_bytes(root: &Path, raw: &[u8]) -> Result<PathBuf, ChangesError> {
    let refuse = |why: &str| ChangesError::Refused(format!("path {} {why}", display(raw)));
    if raw.is_empty() {
        return Err(refuse("is empty"));
    }
    if raw[0] == b'/' || (cfg!(windows) && raw[0] == b'\\') {
        return Err(refuse("is absolute"));
    }
    if cfg!(windows) && raw.len() >= 2 && raw[1] == b':' {
        return Err(refuse("has a drive prefix"));
    }
    let mut joined = root.to_path_buf();
    for part in raw.split(|b| *b == b'/') {
        if part.is_empty() || part == b"." || part == b".." {
            return Err(refuse("has an empty, '.' or '..' component"));
        }
        if part.eq_ignore_ascii_case(b".git") {
            return Err(refuse("points into .git"));
        }
        joined.push(component(part).ok_or_else(|| refuse("is not a valid name here"))?);
    }
    Ok(joined)
}

/// Every byte string is a valid Unix name (the `Option` matches the Windows variant).
#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn component(part: &[u8]) -> Option<&std::ffi::OsStr> {
    use std::os::unix::ffi::OsStrExt;
    Some(std::ffi::OsStr::from_bytes(part))
}

#[cfg(not(unix))]
fn component(part: &[u8]) -> Option<&std::ffi::OsStr> {
    let text = std::str::from_utf8(part).ok()?;
    (!text.contains('\\')).then(|| std::ffi::OsStr::new(text))
}

/// The bytes git uses for `path` (relative, `/`-separated): the inverse of joining for paths we read from
/// the disk.
#[cfg(unix)]
#[must_use]
pub fn os_bytes(path: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_bytes().to_vec()
}

/// The bytes git uses for `path` (relative): UTF-8 with `/` separators.
#[cfg(not(unix))]
#[must_use]
pub fn os_bytes(path: &std::ffi::OsStr) -> Vec<u8> {
    path.to_string_lossy().replace('\\', "/").into_bytes()
}
