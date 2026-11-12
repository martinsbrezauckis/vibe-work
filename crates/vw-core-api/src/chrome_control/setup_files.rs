//! Checked registration I/O for setup and repair. Directory handles pin every ancestor; reads refuse links
//! and reparse points, and writes use fresh temporary files followed by a same-directory rename.
use super::ChromeControlConfig;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path};

#[path = "setup_files_retry.rs"]
mod retry;

fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
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

fn parent(config: &ChromeControlConfig, path: &Path, create: bool) -> io::Result<Dir> {
    let folder = path.parent().ok_or_else(refused)?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if config.is_macos()
        && folder
            .file_name()
            .is_some_and(|name| name == "NativeMessagingHosts")
    {
        return native_host_parent(config, folder, create);
    }
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
    let mut current = root.clone();
    access(config, Access::ReadDir, &current)?;
    let mut dir = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    for part in parts {
        let Component::Normal(name) = part else {
            return Err(refused());
        };
        current.push(name);
        if create {
            access(config, Access::CreateDir, &current)?;
            match dir.create_dir(name) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        access(config, Access::ReadDir, &current)?;
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

pub(super) fn plain_file(config: &ChromeControlConfig, path: &Path) -> io::Result<bool> {
    let dir = parent(config, path, false)?;
    Ok(open_at(&dir, Path::new(path.file_name().ok_or_else(refused)?))?.is_some())
}

/// Compares binary payloads through the checked parent without the registration JSON size limit.
pub(super) fn matches(
    config: &ChromeControlConfig,
    path: &Path,
    expected: &[u8],
) -> io::Result<bool> {
    let dir = parent(config, path, false)?;
    let Some(mut file) = open_at(&dir, Path::new(path.file_name().ok_or_else(refused)?))? else {
        return Ok(false);
    };
    if file.metadata()?.len() != expected.len() as u64 {
        return Ok(false);
    }
    let mut offset = 0;
    let mut chunk = [0; 8192];
    loop {
        let count = file.read(&mut chunk)?;
        if count == 0 {
            return Ok(offset == expected.len());
        }
        if expected.get(offset..offset + count) != Some(&chunk[..count]) {
            return Ok(false);
        }
        offset += count;
    }
}

pub(super) fn read(config: &ChromeControlConfig, path: &Path) -> io::Result<Option<Vec<u8>>> {
    let dir = match parent(config, path, false) {
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
pub(super) fn write(
    config: &ChromeControlConfig,
    path: &Path,
    before: Option<&[u8]>,
    bytes: &[u8],
) -> io::Result<()> {
    let dir = parent(config, path, true)?;
    let name = Path::new(path.file_name().ok_or_else(refused)?);
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|e| io::Error::other(e.to_string()))?;
    let temp = format!(".browser-host-{:032x}.new", u128::from_le_bytes(random));
    let result = (|| {
        let mut file = dir.open_with(&temp, OpenOptions::new().write(true).create_new(true))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let unchanged = || {
            if read_at(&dir, name, before.map_or(1024 * 1024, <[u8]>::len))?.as_deref() != before {
                return Err(refused());
            }
            Ok(())
        };
        if before.is_none() {
            unchanged()?;
            dir.hard_link(&temp, &dir, name)?;
            dir.remove_file(&temp)
        } else {
            retry::replace(
                cfg!(windows),
                unchanged,
                || dir.rename(&temp, &dir, name),
                std::thread::sleep,
            )
        }
    })();
    if result.is_err() {
        let _ = dir.remove_file(&temp);
    }
    result
}

/// Removes only an unchanged plain file through its checked parent.
pub(super) fn remove(config: &ChromeControlConfig, path: &Path, expected: &[u8]) -> io::Result<()> {
    let dir = match parent(config, path, false) {
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

/// Operations observed by the permission fixture before reaching the operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::chrome_control) enum Access {
    ReadDir,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Traverse,
    CreateDir,
}
#[cfg(test)]
pub(in crate::chrome_control) type AccessCheck =
    std::sync::Arc<dyn Fn(Access, &Path) -> io::Result<()> + Send + Sync>;

// The injected permission boundary can fail; the production no-op keeps the same signature.
#[allow(clippy::unnecessary_wraps)]
pub(in crate::chrome_control) fn access(
    config: &ChromeControlConfig,
    op: Access,
    path: &Path,
) -> io::Result<()> {
    #[cfg(test)]
    if let Some(check) = &config.file_access {
        return check(op, path);
    }
    let _ = (config, op, path);
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn native_host_parent(
    config: &ChromeControlConfig,
    folder: &Path,
    create: bool,
) -> io::Result<Dir> {
    use rustix::fs::{Mode, OFlags};
    if !folder.is_absolute()
        || folder
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(refused());
    }
    let data = folder.parent().ok_or_else(refused)?;
    // Search-only is essential: a read handle to Chrome itself triggers app-data protection.
    // The kernel rejects links in every ancestor, so mkdirat cannot be redirected elsewhere.
    access(config, Access::Traverse, data)?;
    let parent = vw_platform::search_directory::open(data).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound
            && let Some(browser) = super::places::browser_of(data)
        {
            return io::Error::new(
                io::ErrorKind::NotFound,
                format!("Open {browser} once, then retry Connect."),
            );
        }
        error
    })?;
    if create {
        access(config, Access::CreateDir, folder)?;
        match rustix::fs::mkdirat(&parent, "NativeMessagingHosts", Mode::RWXU) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
    }
    access(config, Access::ReadDir, folder)?;
    let host = rustix::fs::openat(
        &parent,
        "NativeMessagingHosts",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    Ok(Dir::from_std_file(std::fs::File::from(host)))
}
