//! Checked registration I/O for setup and repair. Directory handles pin every ancestor; reads refuse links
//! and reparse points, and writes use fresh temporary files followed by a same-directory rename.
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path};

fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Browser setup path is not a plain file or folder",
    )
}

fn linked(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

fn parent(path: &Path, create: bool) -> io::Result<Dir> {
    let folder = path.parent().ok_or_else(refused)?;
    if !folder.is_absolute() {
        return Err(refused());
    }
    let mut root = std::path::PathBuf::new();
    let mut parts = folder.components().peekable();
    while matches!(
        parts.peek(),
        Some(Component::Prefix(_) | Component::RootDir)
    ) {
        root.push(parts.next().ok_or_else(refused)?.as_os_str());
    }
    let mut dir = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    for part in parts {
        let Component::Normal(name) = part else {
            return Err(refused());
        };
        if create {
            match dir.create_dir(name) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        let meta = dir.symlink_metadata(name)?;
        if linked(&meta) || !meta.is_dir() {
            return Err(refused());
        }
        dir = dir.open_dir_nofollow(name)?;
        if linked(&dir.dir_metadata()?) {
            return Err(refused());
        }
    }
    Ok(dir)
}

fn open_at(dir: &Dir, name: &Path) -> io::Result<Option<cap_std::fs::File>> {
    let meta = match dir.symlink_metadata(name) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if linked(&meta) || !meta.is_file() {
        return Err(refused());
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let file = dir.open_with(name, &options)?;
    let meta = file.metadata()?;
    if linked(&meta) || !meta.is_file() {
        return Err(refused());
    }
    Ok(Some(file))
}

fn read_at(dir: &Dir, name: &Path, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let Some(file) = open_at(dir, name)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("Browser setup file is too large"));
    }
    Ok(Some(bytes))
}

pub(super) fn plain_file(path: &Path) -> io::Result<bool> {
    let dir = parent(path, false)?;
    Ok(open_at(&dir, Path::new(path.file_name().ok_or_else(refused)?))?.is_some())
}

pub(super) fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let dir = match parent(path, false) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    read_at(
        &dir,
        Path::new(path.file_name().ok_or_else(refused)?),
        1024 * 1024,
    )
}

/// Rechecks inspected contents immediately before replacement; creation never replaces an existing entry.
pub(super) fn write(path: &Path, before: Option<&[u8]>, bytes: &[u8]) -> io::Result<()> {
    let dir = parent(path, true)?;
    let name = Path::new(path.file_name().ok_or_else(refused)?);
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|e| io::Error::other(e.to_string()))?;
    let temp = format!(".browser-host-{:032x}.new", u128::from_le_bytes(random));
    let result = (|| {
        let mut file = dir.open_with(&temp, OpenOptions::new().write(true).create_new(true))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if read_at(&dir, name, before.map_or(1024 * 1024, <[u8]>::len))?.as_deref() != before {
            return Err(refused());
        }
        if before.is_none() {
            dir.hard_link(&temp, &dir, name)?;
            dir.remove_file(&temp)
        } else {
            dir.rename(&temp, &dir, name)
        }
    })();
    if result.is_err() {
        let _ = dir.remove_file(&temp);
    }
    result
}

/// Removes only an unchanged plain file through its checked parent.
pub(super) fn remove(path: &Path, expected: &[u8]) -> io::Result<()> {
    let dir = match parent(path, false) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let name = Path::new(path.file_name().ok_or_else(refused)?);
    match read_at(&dir, name, expected.len())? {
        None => Ok(()),
        Some(bytes) if bytes == expected => dir.remove_file(name),
        Some(_) => Err(refused()),
    }
}
