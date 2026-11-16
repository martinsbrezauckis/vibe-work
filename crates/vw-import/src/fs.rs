//! The only way the scan touches the disk: a reader that refuses, before opening anything, every file that can
//! hold a sign-in or a secret store (`.env*`, `auth.json`, `credentials.json`, `.credentials.json`, the `~/.claude/`
//! agent's `~/.claude.json` account file, key and certificate files), never follows a link, and reads at most
//! [`MAX_FILE_BYTES`] per file and [`MAX_ITEMS`] directory entries per folder.
//!
//! Depends on: std. Called by: the agent readers (`agents`). Tests wrap it to record every path opened.

use std::path::{Path, PathBuf};

/// The largest file the scan reads.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// The most entries the scan lists in one folder.
pub const MAX_ITEMS: usize = 2_000;

/// One folder entry: its path and whether it is a plain folder (links are neither file nor folder here).
#[derive(Debug, Clone)]
pub struct Entry {
    /// The entry's path.
    pub path: PathBuf,
    /// A folder, not a link.
    pub dir: bool,
    /// A plain file, not a link.
    pub file: bool,
}

/// Disk access for the scan; the real one is [`Disk`].
pub trait Fs {
    /// The bytes of a plain file (not a link), at most [`MAX_FILE_BYTES`]; `None` when it is missing, a link,
    /// too large or unreadable.
    fn read(&self, path: &Path) -> Option<Vec<u8>>;
    /// The entries of a plain folder, at most [`MAX_ITEMS`], sorted by name; empty when it is missing or a link.
    fn list(&self, path: &Path) -> Vec<Entry>;
    /// Whether something exists at `path` (no read, no link followed).
    fn exists(&self, path: &Path) -> bool;
}

/// A file the scan never opens, whatever folder it is in.
#[must_use]
pub fn never_read(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".env")
        || matches!(
            lower.as_str(),
            "auth.json"
                | "credentials.json"
                | ".credentials.json"
                | ".claude.json"
                | ".netrc"
                | "keychain"
                | "id_rsa"
                | "id_ed25519"
        )
        || [".pem", ".key", ".p12", ".pfx", ".keychain-db"]
            .iter()
            .any(|ext| lower.ends_with(ext))
}

/// The real disk, with the refusals of [`never_read`] and no link following.
#[derive(Debug, Default, Clone, Copy)]
pub struct Disk;

impl Fs for Disk {
    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        if never_read(path) {
            return None;
        }
        let meta = std::fs::symlink_metadata(path).ok()?;
        if !meta.file_type().is_file() || meta.len() > MAX_FILE_BYTES {
            return None;
        }
        let bytes = std::fs::read(path).ok()?;
        (u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= MAX_FILE_BYTES).then_some(bytes)
    }

    fn list(&self, path: &Path) -> Vec<Entry> {
        let plain = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir());
        if !plain {
            return Vec::new();
        }
        let Ok(read) = std::fs::read_dir(path) else {
            return Vec::new();
        };
        let mut entries: Vec<Entry> = read
            .filter_map(Result::ok)
            .take(MAX_ITEMS)
            .filter_map(|e| {
                let kind = e.file_type().ok()?;
                Some(Entry {
                    path: e.path(),
                    dir: kind.is_dir(),
                    file: kind.is_file(),
                })
            })
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    fn exists(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }
}
