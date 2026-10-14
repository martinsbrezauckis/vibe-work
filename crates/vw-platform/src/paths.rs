//! The everyday spelling of a path: on Windows `std::fs::canonicalize` answers the verbatim form
//! `\\?\C:\x`, which no user types, the File tab showed as is (Windows 11, `vw_files_core`) and the chat's path rules
//! refuse as a device path when an agent repeats it. Every path a person or an agent reads goes through
//! [`plain_path`] (or [`canonical_plain`]); containment checks keep comparing their own normalized keys and are not
//! affected. (Measured on Windows 11: a verbatim working folder given to `Command::current_dir` is NOT a problem, Windows
//! hands the child the plain form; so this is about what is shown.)
//!
//! Depends on: `dunce` (pure string rules, no I/O; already in the lockfile through Tauri). Called by:
//! vw-core-api (`files` display, the agent tools' working folders), tests.
//! Invariants:
//! - The prefix is dropped only when the plain form names the same file (`dunce`'s rules: a drive path, under
//!   the legacy length limit, no reserved device name such as `CON` or `COM1`, no name ending in a dot or space);
//!   otherwise the path comes back unchanged, verbatim prefix included.
//! - Network forms (`\\?\UNC\server\share`) stay as they are.
//! - Elsewhere than Windows both functions are the identity / `std::fs::canonicalize`.

use std::path::{Path, PathBuf};

/// `path` without the Windows verbatim prefix when that names the same file; otherwise `path` unchanged.
/// No filesystem access.
#[must_use]
pub fn plain_path(path: &Path) -> PathBuf {
    dunce::simplified(path).to_path_buf()
}

/// `std::fs::canonicalize`, then [`plain_path`].
///
/// # Errors
/// The filesystem error of `canonicalize` (missing path, no access).
pub fn canonical_plain(path: &Path) -> std::io::Result<PathBuf> {
    path.canonicalize().map(|p| plain_path(&p))
}

/// How folder names compare on a file system family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderFold {
    /// Windows: case-insensitive names, `/` and `\` both separators, `\\?\` verbatim prefixes.
    Windows,
    /// macOS: case-insensitive by default (APFS, HFS+); `\` is an ordinary name character there.
    Mac,
    /// Linux and other Unix: byte-exact.
    Exact,
}

impl FolderFold {
    /// The rule of the platform this was built for.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Exact
        }
    }
}

/// Windows path text in one spelling: `/` as `\`, the verbatim prefix dropped (`\\?\C:\x` -> `c:\x`,
/// `\\?\UNC\s\x` -> `\\s\x`), lower case. Pure string logic, so it is tested on every platform. The one Windows
/// path-key rule of the repository (used by Changes and project folders).
#[must_use]
pub fn windows_key(text: &str) -> String {
    let text = text.replace('/', "\\");
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        text
    };
    plain.to_lowercase()
}

/// The form in which two spellings of one folder compare equal under `fold` (`C:\Dev\app`,
/// `c:/dev/app/` and `\\?\C:\Dev\app` are one folder on Windows; `/Users/me/App` and `/Users/me/app` on a default
/// macOS volume). Trailing separators are dropped (a root such as `/` or `C:\` keeps its own). Only for comparing:
/// what is shown keeps the spelling it came with. A case-sensitive macOS volume (rare, opt-in) would see two
/// folders that differ only in case as one; the cost is that they share one project group.
#[must_use]
pub fn folder_key_for(text: &str, fold: FolderFold) -> String {
    let trimmed = |t: &str, seps: &[char]| -> String {
        let cut = t.trim_end_matches(seps);
        if cut.is_empty() || cut.ends_with(':') {
            t.to_owned()
        } else {
            cut.to_owned()
        }
    };
    match fold {
        FolderFold::Windows => trimmed(&windows_key(text), &['\\']),
        FolderFold::Mac => trimmed(text, &['/']).to_lowercase(),
        FolderFold::Exact => trimmed(text, &['/']),
    }
}

/// [`folder_key_for`] with this platform's rule.
#[must_use]
pub fn folder_key(text: &str) -> String {
    folder_key_for(text, FolderFold::current())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_are_unchanged_and_verbatim_drive_paths_lose_the_prefix_on_windows() {
        assert_eq!(plain_path(Path::new("/tmp/a b")), PathBuf::from("/tmp/a b"));
        if cfg!(windows) {
            assert_eq!(
                plain_path(Path::new(r"\\?\C:\Users\me\a.txt")),
                PathBuf::from(r"C:\Users\me\a.txt")
            );
            // A reserved device name or a trailing dot would name something else without the prefix.
            for kept in [
                r"\\?\C:\x\COM1",
                r"\\?\C:\x\name.",
                r"\\?\UNC\server\share\x",
            ] {
                assert_eq!(plain_path(Path::new(kept)), PathBuf::from(kept));
            }
        }
    }
}
