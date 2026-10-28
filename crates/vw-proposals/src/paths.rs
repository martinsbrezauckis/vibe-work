//! One filesystem path-validation pass at capture, repeated before approval.
//! Uses Path components, `symlink_metadata` and parent canonicalization; no string scanning.

use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::Error;

pub(crate) struct Target {
    pub(crate) absolute: PathBuf,
    pub(crate) metadata: Option<Metadata>,
}

pub(crate) fn normalize(path: &Path) -> Result<PathBuf, Error> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => return Err(Error::InvalidPath { path: path.into() }),
        }
    }
    if relative.as_os_str().is_empty() || relative.to_str().is_none() {
        return Err(Error::InvalidPath { path: path.into() });
    }
    Ok(relative)
}

pub(crate) fn absolute_root(root: &Path) -> Result<PathBuf, Error> {
    if root.is_absolute() {
        Ok(root.to_owned())
    } else {
        Ok(std::env::current_dir()
            .map_err(|error| Error::io(root, error))?
            .join(root))
    }
}

pub(crate) fn validate_root(root: &Path) -> Result<PathBuf, Error> {
    for ancestor in root.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|error| Error::io(root, error))?;
        if metadata.file_type().is_symlink() {
            return Err(Error::Symlink {
                path: ancestor.into(),
            });
        }
        if ancestor == root && !metadata.is_dir() {
            return Err(Error::WrongType {
                path: root.into(),
                expected: "directory",
            });
        }
    }
    fs::canonicalize(root).map_err(|error| Error::io(root, error))
}

pub(crate) fn validate_target_with_parents(
    root: &Path,
    relative: &Path,
    allow_missing: bool,
) -> Result<Target, Error> {
    let mut absolute = root.to_owned();
    let mut metadata = None;
    for component in relative.components() {
        absolute.push(component);
        metadata = match fs::symlink_metadata(&absolute) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(Error::Symlink {
                        path: relative.into(),
                    });
                }
                Some(metadata)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(Error::io(relative, error)),
        };
    }
    // The normalized path has a filename. Requiring its immediate parent to exist keeps
    // proposals limited to file operations, without creating unreviewed directories.
    let parent = absolute.parent().ok_or_else(|| Error::InvalidPath {
        path: relative.into(),
    })?;
    let mut existing = parent;
    if allow_missing {
        while !existing.exists() {
            existing = existing.parent().ok_or_else(|| Error::InvalidPath {
                path: relative.into(),
            })?;
        }
    }
    let parent = fs::canonicalize(existing).map_err(|error| Error::io(relative, error))?;
    if !parent.starts_with(root) {
        return Err(Error::OutsideRoot {
            path: relative.into(),
        });
    }
    Ok(Target { absolute, metadata })
}
