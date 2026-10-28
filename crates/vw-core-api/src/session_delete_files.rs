//! Exact Vibe deletion targets, inspected and removed through no-follow directory handles.
//! Depends on cap-std/cap-fs-ext; `session_delete` supplies configured storage and authorized UUIDs.
//! Refuses every link/reparse point, including ancestors and descendants; never chmods or follows one.

use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Targets {
    pub(crate) save_dir: PathBuf,
    pub(crate) prefix: String,
    /// Captured before Vibe can unlink meta.json; never reconstructed from an error message.
    pub(crate) legacy: Option<String>,
}

pub(crate) fn uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "chat deletion refused an unsafe storage path",
    )
}

fn plain(meta: &Metadata) -> io::Result<()> {
    #[cfg(windows)]
    let linked = {
        use cap_std::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = meta.file_type().is_symlink();
    if linked { Err(refused()) } else { Ok(()) }
}

fn child(parent: &Dir, name: &Path) -> io::Result<Dir> {
    let meta = parent.symlink_metadata(name)?;
    plain(&meta)?;
    if !meta.is_dir() {
        return Err(refused());
    }
    let dir = parent.open_dir_nofollow(name)?;
    plain(&dir.dir_metadata()?)?;
    Ok(dir)
}

/// Opens from the volume root, so no ancestor can be silently resolved through a link.
pub(crate) fn root(path: &Path) -> io::Result<Dir> {
    if !path.is_absolute() {
        return Err(refused());
    }
    let mut anchor = PathBuf::new();
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                #[cfg(windows)]
                if !matches!(
                    prefix.kind(),
                    std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                ) {
                    return Err(refused());
                }
                anchor.push(prefix.as_os_str());
            }
            Component::RootDir => anchor.push(component.as_os_str()),
            Component::Normal(name) => names.push(name),
            _ => return Err(refused()),
        }
    }
    let mut dir = Dir::open_ambient_dir(anchor, cap_std::ambient_authority())?;
    plain(&dir.dir_metadata()?)?;
    for name in names {
        dir = child(&dir, Path::new(name))?;
    }
    Ok(dir)
}

fn legacy_name(name: &str, prefix: &str, id: &str) -> bool {
    !prefix.contains(['/', '\\', ':', '*', '?', '[', ']'])
        && !name.contains(['/', '\\', ':'])
        && name.starts_with(&format!("{prefix}_"))
        && name.ends_with(&format!("_{}", &id[..8]))
        && Path::new(name).components().count() == 1
}

fn metadata_id(dir: &Dir) -> io::Result<Option<String>> {
    let name = Path::new("meta.json");
    // Legacy SessionLoader.METADATA_FILENAME is meta.json in Vibe 3.0.2.
    let meta = match dir.symlink_metadata(name) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    plain(&meta)?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return Err(refused());
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = dir.open_with(name, &options)?;
    plain(&file.metadata()?)?;
    let mut bytes = Vec::new();
    file.by_ref().take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    Ok(value["session_id"].as_str().map(str::to_owned))
}

pub(crate) fn has_unified(targets: &Targets, id: &str) -> io::Result<bool> {
    if !uuid(id) {
        return Err(refused());
    }
    let check = || -> io::Result<bool> {
        let dir = root(&targets.save_dir)?;
        let unified = child(&dir, Path::new("unified"))?;
        let meta = unified.symlink_metadata(id)?;
        plain(&meta)?;
        Ok(true)
    };
    match check() {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        result => result,
    }
}

/// Resolve the same full-id-checked legacy folder as `delete_saved_session`, before Vibe damages it.
pub(crate) fn capture(targets: &mut Targets, id: &str) -> io::Result<()> {
    if !uuid(id)
        || targets
            .prefix
            .contains(['/', '\\', ':', '*', '?', '[', ']'])
    {
        return Err(refused());
    }
    let dir = match root(&targets.save_dir) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if let Some(name) = &targets.legacy {
        if !legacy_name(name, &targets.prefix, id) {
            return Err(refused());
        }
        if let Ok(candidate) = child(&dir, Path::new(name))
            && metadata_id(&candidate)?.is_some_and(|actual| actual != id)
        {
            return Err(refused());
        }
        return Ok(());
    }
    for entry in dir.entries()? {
        let name = entry?.file_name();
        let Some(name) = name.to_str() else { continue };
        if legacy_name(name, &targets.prefix, id) {
            let candidate = child(&dir, Path::new(name))?;
            if metadata_id(&candidate)?.as_deref() == Some(id) {
                targets.legacy = Some(name.to_owned());
                break;
            }
        }
    }
    Ok(())
}

fn inspect(parent: &Dir, name: &Path, budget: &mut usize, depth: usize) -> io::Result<()> {
    if *budget == 0 || depth > 128 {
        return Err(refused());
    }
    *budget -= 1;
    let meta = match parent.symlink_metadata(name) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    plain(&meta)?;
    if meta.is_dir() {
        let dir = child(parent, name)?;
        for entry in dir.entries()? {
            inspect(&dir, Path::new(&entry?.file_name()), budget, depth + 1)?;
        }
    } else if !meta.is_file() || depth == 0 {
        return Err(refused());
    }
    Ok(())
}

fn remove(parent: &Dir, name: &Path, budget: &mut usize, depth: usize) -> io::Result<()> {
    if *budget == 0 || depth > 128 {
        return Err(refused());
    }
    *budget -= 1;
    let meta = match parent.symlink_metadata(name) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    plain(&meta)?;
    if meta.is_dir() {
        let dir = child(parent, name)?;
        for entry in dir.entries()? {
            remove(&dir, Path::new(&entry?.file_name()), budget, depth + 1)?;
        }
        drop(dir);
        parent.remove_dir(name)
    } else if meta.is_file() && depth > 0 {
        parent.remove_file(name)
    } else {
        Err(refused())
    }
}

/// Preflight both trees before any removal; the mutating pass checks again and uses pinned parents.
pub(crate) fn finish(targets: &Targets, id: &str) -> io::Result<()> {
    if !uuid(id)
        || targets
            .prefix
            .contains(['/', '\\', ':', '*', '?', '[', ']'])
    {
        return Err(refused());
    }
    let dir = match root(&targets.save_dir) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let unified = match child(&dir, Path::new("unified")) {
        Ok(dir) => Some(dir),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let mut budget = 100_000;
    if let Some(unified) = &unified {
        inspect(unified, Path::new(id), &mut budget, 0)?;
    }
    if let Some(name) = &targets.legacy {
        if !legacy_name(name, &targets.prefix, id) {
            return Err(refused());
        }
        inspect(&dir, Path::new(name), &mut budget, 0)?;
    }
    let mut budget = 100_000;
    if let Some(unified) = &unified {
        remove(unified, Path::new(id), &mut budget, 0)?;
    }
    if let Some(name) = &targets.legacy {
        remove(&dir, Path::new(name), &mut budget, 0)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_delete_files_tests.rs"]
mod tests;
