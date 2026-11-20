//! The only way the scan touches the disk: a reader that refuses, before opening anything, every file that can
//! hold a sign-in or a secret store (`.env*`, `auth.json`, `credentials.json`, `.credentials.json`, the `~/.claude/`
//! agent's `~/.claude.json` account file, key and certificate files), never follows a link, and reads at most
//! [`MAX_FILE_BYTES`] per file and [`MAX_ITEMS`] directory entries per folder.
//!
//! Depends on: cap-std and cap-fs-ext. Called by the agent readers (`agents`).

use std::ffi::OsString;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};

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
    /// A link or Windows reparse point that the scan refused.
    pub linked: bool,
}

/// An import path was refused because a component is a link or reparse point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkedPath;

/// Disk access for the scan; the real one is [`Disk`].
pub trait Fs {
    /// The bytes of a plain file (not a link), at most [`MAX_FILE_BYTES`]; `None` when it is missing, a link,
    /// too large or unreadable.
    fn read(&self, path: &Path) -> Option<Vec<u8>>;
    /// The same read with a distinct linked-path result for a visible scan row.
    ///
    /// # Errors
    /// Returns [`LinkedPath`] when a path component is a link or reparse point.
    fn read_checked(&self, path: &Path) -> Result<Option<Vec<u8>>, LinkedPath> {
        Ok(self.read(path))
    }
    /// The entries of a plain folder, at most [`MAX_ITEMS`], sorted by name; empty when it is missing or a link.
    fn list(&self, path: &Path) -> Vec<Entry>;
    /// The same listing with a distinct linked-path result for a visible scan row.
    ///
    /// # Errors
    /// Returns [`LinkedPath`] when a path component is a link or reparse point.
    fn list_checked(&self, path: &Path) -> Result<Vec<Entry>, LinkedPath> {
        Ok(self.list(path))
    }
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

#[derive(Debug)]
enum WalkError {
    Linked,
    Other,
}

fn linked(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// The anchor is an OS root; every named component is opened from its parent handle.
fn parent(path: &Path) -> Result<(Dir, OsString), WalkError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| WalkError::Other)?
            .join(path)
    };
    let mut anchor = PathBuf::new();
    let mut names = Vec::new();
    for part in absolute.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => anchor.push(part.as_os_str()),
            Component::Normal(name) => names.push(name.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => return Err(WalkError::Other),
        }
    }
    if !anchor.is_absolute() || names.is_empty() {
        return Err(WalkError::Other);
    }
    let mut dir = Dir::open_ambient_dir(anchor, cap_std::ambient_authority())
        .map_err(|_| WalkError::Other)?;
    let leaf = names.pop().ok_or(WalkError::Other)?;
    for name in names {
        let meta = dir.symlink_metadata(&name).map_err(|_| WalkError::Other)?;
        if linked(&meta) {
            return Err(WalkError::Linked);
        }
        if !meta.is_dir() {
            return Err(WalkError::Other);
        }
        dir = match dir.open_dir_nofollow(&name) {
            Ok(opened) => opened,
            Err(_) if dir.symlink_metadata(&name).is_ok_and(|meta| linked(&meta)) => {
                return Err(WalkError::Linked);
            }
            Err(_) => return Err(WalkError::Other),
        };
        if linked(&dir.dir_metadata().map_err(|_| WalkError::Other)?) {
            return Err(WalkError::Linked);
        }
    }
    Ok((dir, leaf))
}

fn leaf(path: &Path) -> Result<(Dir, OsString, Metadata), WalkError> {
    let (dir, name) = parent(path)?;
    let meta = dir.symlink_metadata(&name).map_err(|_| WalkError::Other)?;
    if linked(&meta) {
        return Err(WalkError::Linked);
    }
    Ok((dir, name, meta))
}

fn read_checked(path: &Path) -> Result<Option<Vec<u8>>, LinkedPath> {
    if never_read(path) {
        return Ok(None);
    }
    let (dir, name, meta) = match leaf(path) {
        Ok(parts) => parts,
        Err(WalkError::Linked) => return Err(LinkedPath),
        Err(WalkError::Other) => return Ok(None),
    };
    if !meta.is_file() {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let file = match dir.open_with(&name, &options) {
        Ok(file) => file,
        Err(_) if dir.symlink_metadata(&name).is_ok_and(|meta| linked(&meta)) => {
            return Err(LinkedPath);
        }
        Err(_) => return Ok(None),
    };
    let Ok(opened) = file.metadata() else {
        return Ok(None);
    };
    if linked(&opened) {
        return Err(LinkedPath);
    }
    if !opened.is_file() || opened.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    if file
        .into_std()
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Ok(None);
    }
    Ok((bytes.len() as u64 <= MAX_FILE_BYTES).then_some(bytes))
}

fn list_checked(path: &Path) -> Result<Vec<Entry>, LinkedPath> {
    let (parent, name, meta) = match leaf(path) {
        Ok(parts) => parts,
        Err(WalkError::Linked) => return Err(LinkedPath),
        Err(WalkError::Other) => return Ok(Vec::new()),
    };
    if !meta.is_dir() {
        return Ok(Vec::new());
    }
    let dir = match parent.open_dir_nofollow(&name) {
        Ok(dir) => dir,
        Err(_)
            if parent
                .symlink_metadata(&name)
                .is_ok_and(|meta| linked(&meta)) =>
        {
            return Err(LinkedPath);
        }
        Err(_) => return Ok(Vec::new()),
    };
    let Ok(opened) = dir.dir_metadata() else {
        return Ok(Vec::new());
    };
    if linked(&opened) {
        return Err(LinkedPath);
    }
    let Ok(read) = dir.entries() else {
        return Ok(Vec::new());
    };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .take(MAX_ITEMS)
        .filter_map(|entry| {
            let name = entry.file_name();
            let kind = dir.symlink_metadata(&name).ok()?;
            let linked = linked(&kind);
            Some(Entry {
                path: path.join(name),
                dir: !linked && kind.is_dir(),
                file: !linked && kind.is_file(),
                linked,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

impl Fs for Disk {
    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        read_checked(path).ok().flatten()
    }

    fn read_checked(&self, path: &Path) -> Result<Option<Vec<u8>>, LinkedPath> {
        read_checked(path)
    }

    fn list(&self, path: &Path) -> Vec<Entry> {
        list_checked(path).unwrap_or_default()
    }

    fn list_checked(&self, path: &Path) -> Result<Vec<Entry>, LinkedPath> {
        list_checked(path)
    }

    fn exists(&self, path: &Path) -> bool {
        leaf(path).is_ok()
    }
}
