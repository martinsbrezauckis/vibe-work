//! Which file a path in the chat names, and whether the app may show it.
//!
//! A path in agent text is untrusted. The rule ("never act as an oracle for other paths"): a path whose
//! LEXICAL form lies outside every allowed folder is refused before anything touches the disk, with the same answer
//! as a missing file. Only a lexically allowed path is canonicalized; its canonical form must lie inside the
//! canonical allowed folder (a symlink leaving the folder is refused), and it must be a regular file.
//!
//! Allowed folders: the session's own folder (the core's record, never a client value) and the folders the user
//! picked. [`Scope`] can narrow both (no picked folders, no hidden segments). Because the Debug API
//! token holder acts as the user, `files` builds the same full scope for both origins.
//!
//! Depends on: std only (the filesystem through [`ScopeFs`], so a test can count every access). Called by: `files`
//! (resolve, and the re-check before every read or open), tests.
//! Invariants:
//! - [`resolve_in`] / [`recheck_in`] call [`ScopeFs`] only for a candidate whose lexical form is inside an allowed
//!   folder, plus one canonicalization of the session's own folder (the core's record, not client text).
//! - Containment is compared on `path_key` forms (Windows: one spelling, no case), never on raw text.
//! - Every refusal is the same `None`: no reason, no difference between "missing" and "not allowed".
//! - Windows: UNC and device forms (`\\server\share`, `\\?\`, `\\.\`) are refused before any access.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

/// Longest path text considered.
pub const MAX_PATH_TEXT: usize = 4096;

/// The filesystem as the resolver sees it (a test counts the calls).
pub trait ScopeFs {
    /// `std::fs::canonicalize`.
    ///
    /// # Errors
    /// The filesystem error.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    /// `std::fs::metadata` reduced to what is used: `(is_file, len, modified)`.
    ///
    /// # Errors
    /// The filesystem error.
    fn facts(&self, path: &Path) -> io::Result<(bool, u64, Option<SystemTime>)>;
}

/// The real filesystem.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

impl ScopeFs for RealFs {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        path.canonicalize()
    }
    fn facts(&self, path: &Path) -> io::Result<(bool, u64, Option<SystemTime>)> {
        let meta = std::fs::metadata(path)?;
        Ok((meta.is_file(), meta.len(), meta.modified().ok()))
    }
}

/// Where a session's paths may point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The session's folder as the core recorded it (relative paths are joined to it).
    pub session: PathBuf,
    /// Folders the user picked (canonical).
    pub picked: Vec<PathBuf>,
    /// The user's home folder, for `~/` paths (`None`: `~` is never expanded).
    pub home: Option<PathBuf>,
    /// Hidden segments (`.env`, `.git/...`) allowed below the folder (`files` allows them for both origins).
    pub hidden: bool,
}

/// A file the scope allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The canonical file.
    pub canonical: PathBuf,
    /// The canonical folder it lies in (the session's or a picked one).
    pub root: PathBuf,
    /// True when `root` is the session's own folder.
    pub in_session: bool,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, when the OS reports one.
    pub modified: Option<SystemTime>,
}

/// True for Windows UNC and device forms (also `//server/share`): never touched.
fn network_or_device(text: &str) -> bool {
    text.starts_with("\\\\") || text.starts_with("//")
}

/// A Windows drive path (`C:\x`, `C:/x`).
fn drive_path(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// True when `text` is longer than the scheme and starts with `file://` in any letter case.
///
/// Compares BYTES, never a `&str` slice: `text[..7]` panics when byte 7 falls inside a multi-byte character, and
/// a path in agent text is untrusted (`āāāā.rs`, `Привет.txt` and `🙂🙂.txt` all have their 7th byte inside a
/// character). Slicing the byte view cannot panic, and the scheme is pure ASCII, so the comparison is exact.
fn has_file_scheme(text: &str) -> bool {
    const SCHEME: &[u8] = b"file://";
    text.len() > SCHEME.len() && text.as_bytes()[..SCHEME.len()].eq_ignore_ascii_case(SCHEME)
}

/// Percent-decodes a `file://` path; `None` for a host part, a bad escape or invalid UTF-8.
fn file_url_path(text: &str) -> Option<String> {
    let rest = text
        .strip_prefix("file://")
        .or_else(|| text.strip_prefix("FILE://"))?;
    // `file:///abs` has an empty host; anything else names a host (a network share).
    let path = rest.strip_prefix('/').map(|p| format!("/{p}"))?;
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = char::from(*bytes.get(i + 1)?).to_digit(16)?;
            let lo = char::from(*bytes.get(i + 2)?).to_digit(16)?;
            out.push(u8::try_from(hi * 16 + lo).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(out).ok()?;
    // `file:///C:/x` on Windows.
    let trimmed = decoded.strip_prefix('/').filter(|p| drive_path(p));
    Some(trimmed.map_or(decoded.clone(), str::to_owned))
}

/// Normalizes `.` and `..` without the filesystem; `None` when `..` would climb above the root or the path is
/// not absolute.
#[must_use]
pub fn lexical(path: &Path) -> Option<PathBuf> {
    let mut base = PathBuf::new();
    let mut names: Vec<&std::ffi::OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => base.push(component.as_os_str()),
            Component::Normal(name) => names.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                names.pop()?;
            }
        }
    }
    let out = names.iter().fold(base, |p, n| p.join(n));
    out.is_absolute().then_some(out)
}

/// The absolute, lexically normalized form of `text` for `scope`, or `None` (see the module rules).
#[must_use]
pub fn candidate(scope: &Scope, text: &str) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_PATH_TEXT || text.chars().any(char::is_control) {
        return None;
    }
    let text = if has_file_scheme(text) {
        file_url_path(text)?
    } else {
        text.to_owned()
    };
    if network_or_device(&text) {
        return None;
    }
    // A Windows drive path means nothing on Unix (it would join below the session folder as a name).
    if !cfg!(windows) && drive_path(&text) {
        return None;
    }
    let joined = if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        scope.home.as_ref()?.join(rest)
    } else if text.starts_with('~') {
        return None;
    } else {
        let path = Path::new(&text);
        if path.is_absolute() {
            path.to_path_buf()
        } else if cfg!(windows) && (text.starts_with('\\') || text.starts_with('/')) {
            // A rooted path without a drive on Windows is relative to the current drive: not ours to guess.
            return None;
        } else {
            scope.session.join(path)
        }
    };
    lexical(&joined)
}

/// The comparison form of a path: on Windows one spelling per path (verbatim prefix dropped, `/` as `\`, lower
/// case; `changes_facts::path_key`), elsewhere the path itself. Every containment check below compares keys, so
/// `\\?\C:\x`, `C:/x` and `c:\X` are the same folder and a case difference can never skip a check.
fn key(path: &Path) -> PathBuf {
    crate::changes_facts::path_key(path)
}

/// True when `path` lies in `root` (keys compared).
fn inside(root: &Path, path: &Path) -> bool {
    key(path).starts_with(key(root))
}

/// True when a segment of `path` below `root` is hidden (starts with `.`); keys compared, so it cannot fail open on
/// a spelling difference.
fn hidden_below(root: &Path, path: &Path) -> bool {
    let (root, path) = (key(root), key(path));
    path.strip_prefix(&root).map_or(true, |rest| {
        rest.components()
            .any(|c| matches!(c, Component::Normal(n) if n.to_string_lossy().starts_with('.')))
    })
}

/// Checks an absolute, lexically normal `path` against `scope` (see the module rules).
fn check(scope: &Scope, path: &Path, fs: &dyn ScopeFs) -> Option<Hit> {
    let session_lexical = lexical(&scope.session)?;
    // The session's own folder may be canonicalized (it is the core's record, never client text): an agent prints
    // canonical paths (`pwd`), and a handle stores one. This is the only access before the lexical gate.
    let session_canonical = fs.canonicalize(&session_lexical).ok();
    // Lexical gate: nothing below touches the disk for a path outside every allowed folder.
    let lexical_root = std::iter::once(&session_lexical)
        .chain(session_canonical.iter())
        .chain(scope.picked.iter())
        .find(|root| inside(root, path))?;
    if !scope.hidden && hidden_below(lexical_root, path) {
        return None;
    }
    let canonical = fs.canonicalize(path).ok()?;
    let (root, in_session) = match session_canonical.filter(|r| inside(r, &canonical)) {
        Some(root) => (root, true),
        None => (
            scope
                .picked
                .iter()
                .find(|root| inside(root, &canonical))?
                .clone(),
            false,
        ),
    };
    if !scope.hidden && hidden_below(&root, &canonical) {
        return None;
    }
    let (is_file, size, modified) = fs.facts(&canonical).ok()?;
    is_file.then_some(Hit {
        canonical,
        root,
        in_session,
        size,
        modified,
    })
}

/// Resolves `text` for `scope` through `fs`: the allowed file, or `None` (the same answer for every refusal).
pub fn resolve_in(scope: &Scope, text: &str, fs: &dyn ScopeFs) -> Option<Hit> {
    check(scope, &candidate(scope, text)?, fs)
}

/// [`resolve_in`] on the real filesystem.
#[must_use]
pub fn resolve(scope: &Scope, text: &str) -> Option<Hit> {
    resolve_in(scope, text, &RealFs)
}

/// Checks a path the core stored earlier (a handle's canonical file) again for `scope`, without reading it as chat
/// text (a Windows canonical path starts with `\\?\`, which the text rules refuse as a device path).
pub fn recheck_in(scope: &Scope, path: &Path, fs: &dyn ScopeFs) -> Option<Hit> {
    let path = lexical(path)?;
    check(scope, &path, fs)
}

/// [`recheck_in`] on the real filesystem.
#[must_use]
pub fn recheck(scope: &Scope, path: &Path) -> Option<Hit> {
    recheck_in(scope, path, &RealFs)
}
