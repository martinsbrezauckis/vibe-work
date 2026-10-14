//! The file-system edge of the `.worktreeinclude` copy: opening a source file and creating a destination file
//! without ever following a link.
//!
//! Unix: every path level is opened with `openat(parent_handle, name, O_NOFOLLOW)`, starting from a handle on
//! the checkout (or the new folder). A directory swapped for a link after we looked is not followed because there
//! is no "look" separate from the open: the open itself refuses a link. Files are created with `O_CREAT|O_EXCL|
//! O_NOFOLLOW` (an existing file or link at the leaf is an error, never written through) and their permissions
//! are set on the open handle (`fchmod`), not by path.
//! Elsewhere (Windows): each level is looked at with `symlink_metadata` (links, junctions and any reparse point
//! refused) and the leaf is created with `create_new`; the handle is compared with what was looked at. Residual
//! window there: a same-user process that swaps a directory for a junction between
//! the look and the open (std has no handle-relative open; the Windows equivalent is `NtCreateFile` relative to a
//! directory handle with `FILE_OPEN_REPARSE_POINT`, left for a later block).
//!
//! Depends on: rustix (Unix), `source::is_link_like`. Called by: `copy`, `plan`, `source`.
//! Invariants: no function follows a link at any level; nothing is written outside the folder the caller named;
//! a created file that could not be finished is removed by [`Created::remove`].

use std::fs::{File, Metadata};
use std::path::Path;

use super::SkipReason;

/// A destination file this call created (so it may remove it again).
pub(crate) struct Created {
    /// The open file (write).
    pub(crate) file: File,
    #[cfg(unix)]
    parent: std::os::fd::OwnedFd,
    #[cfg(unix)]
    name: String,
    #[cfg(not(unix))]
    path: std::path::PathBuf,
}

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::os::fd::OwnedFd;
    use std::path::Path;

    use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, mkdirat, openat, statat, unlinkat};
    use rustix::io::Errno;

    use super::{Created, Metadata};
    use crate::include::SkipReason;

    fn open_root(root: &Path) -> Result<OwnedFd, SkipReason> {
        openat(
            CWD,
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| SkipReason::Unreadable)
    }

    /// Why `name` below `parent` could not be opened without following a link.
    fn why(parent: &OwnedFd, name: &str, err: Errno) -> SkipReason {
        match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(st) if FileType::from_raw_mode(st.st_mode) == FileType::Symlink => SkipReason::Link,
            Ok(_) if err == Errno::NOTDIR || err == Errno::LOOP => SkipReason::NotRegular,
            Err(_) if err == Errno::LOOP => SkipReason::Link,
            Ok(_) | Err(_) => SkipReason::Unreadable,
        }
    }

    fn open_dir(parent: &OwnedFd, name: &str) -> Result<OwnedFd, SkipReason> {
        openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| why(parent, name, e))
    }

    pub(super) fn open_source(root: &Path, comps: &[&str]) -> Result<(File, Metadata), SkipReason> {
        let (leaf, dirs) = comps.split_last().ok_or(SkipReason::UnsafeName)?;
        let mut cur = open_root(root)?;
        for name in dirs {
            cur = open_dir(&cur, name)?;
        }
        // NONBLOCK: opening a FIFO must not wait for a writer (it is refused as not regular below).
        let fd = openat(
            &cur,
            *leaf,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|e| why(&cur, leaf, e))?;
        let file = File::from(fd);
        let meta = file.metadata().map_err(|_| SkipReason::Unreadable)?;
        if !meta.is_file() {
            return Err(SkipReason::NotRegular);
        }
        Ok((file, meta))
    }

    pub(super) fn create_dest(dest: &Path, comps: &[&str]) -> Result<Created, SkipReason> {
        let (leaf, dirs) = comps.split_last().ok_or(SkipReason::UnsafeName)?;
        let mut cur = open_root(dest)?;
        for name in dirs {
            cur = match open_dir(&cur, name) {
                Ok(dir) => dir,
                Err(SkipReason::Unreadable) => {
                    // Not there yet (or not openable): make it, then open the new directory the same way.
                    match mkdirat(&cur, *name, Mode::from_raw_mode(0o755)) {
                        Ok(()) | Err(Errno::EXIST) => {}
                        Err(_) => return Err(SkipReason::Unreadable),
                    }
                    open_dir(&cur, name)?
                }
                Err(other) => return Err(other),
            };
        }
        let fd = openat(
            &cur,
            *leaf,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| {
            if e == Errno::EXIST {
                SkipReason::Exists
            } else {
                SkipReason::Unreadable
            }
        })?;
        Ok(Created {
            file: File::from(fd),
            parent: cur,
            name: (*leaf).to_owned(),
        })
    }

    pub(super) fn remove(created: &Created) {
        // Only the file this call created, by its name below the handle it was created in.
        let _ = unlinkat(&created.parent, created.name.as_str(), AtFlags::empty());
    }
}

#[cfg(not(unix))]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::path::{Path, PathBuf};

    use super::{Created, Metadata};
    use crate::include::SkipReason;
    use crate::include::source::is_link_like;

    pub(super) fn open_source(root: &Path, comps: &[&str]) -> Result<(File, Metadata), SkipReason> {
        let mut cur = root.to_path_buf();
        for (i, name) in comps.iter().enumerate() {
            cur.push(name);
            let meta = std::fs::symlink_metadata(&cur).map_err(|_| SkipReason::Unreadable)?;
            if is_link_like(&meta) {
                return Err(SkipReason::Link);
            }
            if i + 1 < comps.len() {
                if !meta.is_dir() {
                    return Err(SkipReason::NotRegular);
                }
            } else {
                if !meta.is_file() {
                    return Err(SkipReason::NotRegular);
                }
                let file = File::open(&cur).map_err(|_| SkipReason::Unreadable)?;
                let opened = file.metadata().map_err(|_| SkipReason::Unreadable)?;
                if opened.len() != meta.len() || opened.modified().ok() != meta.modified().ok() {
                    return Err(SkipReason::Changed);
                }
                return Ok((file, opened));
            }
        }
        Err(SkipReason::UnsafeName)
    }

    pub(super) fn create_dest(dest: &Path, comps: &[&str]) -> Result<Created, SkipReason> {
        let (leaf, dirs) = comps.split_last().ok_or(SkipReason::UnsafeName)?;
        let mut cur = dest.to_path_buf();
        for name in dirs {
            cur.push(name);
            match std::fs::symlink_metadata(&cur) {
                Ok(m) if is_link_like(&m) => return Err(SkipReason::Link),
                Ok(m) if m.is_dir() => {}
                Ok(_) => return Err(SkipReason::Exists),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&cur).map_err(|_| SkipReason::Unreadable)?;
                }
                Err(_) => return Err(SkipReason::Unreadable),
            }
        }
        // Belt and braces: whatever the levels looked like, the parent must resolve below the folder.
        let resolved = cur.canonicalize().map_err(|_| SkipReason::Unreadable)?;
        if !resolved.starts_with(dest) {
            return Err(SkipReason::Link);
        }
        let path: PathBuf = resolved.join(leaf);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    SkipReason::Exists
                } else {
                    SkipReason::Unreadable
                }
            })?;
        Ok(Created { file, path })
    }

    pub(super) fn remove(created: &Created) {
        let _ = std::fs::remove_file(&created.path);
    }
}

/// Opens `root/<comps>` for reading through real directories only; the leaf must be a regular file.
/// The metadata is the open handle's own.
///
/// # Errors
/// `Link` for a link at any level, `NotRegular`, `Unreadable`, `Changed` (swapped while opening, non-Unix).
pub(crate) fn open_source(root: &Path, comps: &[&str]) -> Result<(File, Metadata), SkipReason> {
    imp::open_source(root, comps)
}

/// Creates `dest/<comps>` (parent directories included) for writing; the leaf must not exist.
///
/// # Errors
/// `Link` when a link is in the way, `Exists` when the leaf exists, `Unreadable` otherwise.
pub(crate) fn create_dest(dest: &Path, comps: &[&str]) -> Result<Created, SkipReason> {
    imp::create_dest(dest, comps)
}

impl Created {
    /// Removes the file again (a copy that could not be finished).
    pub(crate) fn remove(self) {
        imp::remove(&self);
    }
}
