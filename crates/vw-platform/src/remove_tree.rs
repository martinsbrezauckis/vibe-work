//! Removes app-owned Vibe trees through directory handles; called by probe and test-home cleanup.
//! Depends on cap-std/cap-fs-ext and Windows handle disposition. Links are never followed or chmodded;
//! shared files keep their permissions, so cleanup cannot make an outside hard link writable.

use std::io;
use std::path::{Component, Path};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;

#[cfg(windows)]
mod windows;

/// Removes a tree strictly below an expected app-owned root, including read-only package copies.
/// The caller must already own and authorize deletion of `path`; this does not widen that scope.
/// Missing paths succeed. Linked ancestors and Windows reparse points are refused. Links within
/// the deleted tree are removed without visiting their targets.
///
/// # Errors
/// Returns filesystem errors, including refusal of paths outside `root`, `..`, and linked ancestors.
pub fn remove_owned_tree(root: &Path, path: &Path) -> io::Result<()> {
    remove_after_root_check(root, path, || {})
}

fn remove_after_root_check(root: &Path, path: &Path, after_check: impl FnOnce()) -> io::Result<()> {
    let relative = path.strip_prefix(root).map_err(|_| boundary())?;
    let parts = relative.components().collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|p| !matches!(p, Component::Normal(_))) {
        return Err(boundary());
    }
    let mut parent = match open_root(root) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let root_meta = parent.dir_metadata()?;
    if linked(&root_meta) || !root_meta.is_dir() {
        return Err(boundary());
    }
    after_check();
    for (index, part) in parts.iter().enumerate() {
        let name = Path::new(part.as_os_str());
        let meta = match parent.symlink_metadata(name) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if index + 1 == parts.len() {
            return remove_entry(&parent, name);
        }
        if linked(&meta) || !meta.is_dir() {
            return Err(boundary());
        }
        parent = parent.open_dir_nofollow(name)?;
    }
    Err(boundary())
}

#[cfg(test)]
mod tests;

fn open_root(root: &Path) -> io::Result<Dir> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY);
    }
    #[cfg(windows)]
    {
        use ::windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0);
    }
    // Normalize trailing separators and dots so the last component itself is opened no-follow.
    // Ancestors may still use platform aliases such as macOS /var; the acquired handle is the boundary.
    options
        .open(root.components().collect::<std::path::PathBuf>())
        .map(Dir::from_std_file)
}

fn boundary() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "cleanup path crosses its ownership boundary",
    )
}

fn linked(meta: &cap_std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        // All reparse points, including junctions, must be treated as links.
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    meta.file_type().is_symlink()
}

fn remove_entry(parent: &Dir, name: &Path) -> io::Result<()> {
    let meta = parent.symlink_metadata(name)?;
    if linked(&meta) {
        #[cfg(windows)]
        {
            use cap_std::fs::MetadataExt;
            if meta.file_attributes() & 0x10 != 0 {
                return parent.remove_dir(name);
            }
        }
        return parent.remove_file(name);
    }
    if meta.is_dir() {
        let dir = parent.open_dir_nofollow(name)?;
        let mut perms = dir.dir_metadata()?.permissions();
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            perms.set_mode(perms.mode() | 0o700);
        }
        #[cfg(windows)]
        perms.set_readonly(false);
        // "." is the already-open directory itself; no path component can be a symlink.
        // cap-std's Linux directory handles may be O_PATH, so File::set_permissions would fail.
        dir.set_permissions(".", perms)?;
        for entry in dir.entries()? {
            remove_entry(&dir, Path::new(&entry?.file_name()))?;
        }
        // Windows refuses to remove a directory while a handle to it is open (sharing violation).
        drop(dir);
        parent.remove_dir(name)
    } else {
        #[cfg(windows)]
        {
            windows::remove_file(parent, name)
        }
        #[cfg(not(windows))]
        {
            parent.remove_file(name)
        }
    }
}
