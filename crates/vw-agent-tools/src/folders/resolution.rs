//! Kernel-order path resolution and Windows prefix normalization for the folder guard.
//! Depends on filesystem metadata and path components; called by Layout and folder command checks.
//! Symlinks are followed before later parent components, including dangling leaf targets.
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
struct ResolutionFailure {
    path: PathBuf,
    cause: std::io::Error,
}

impl std::fmt::Display for ResolutionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "could not resolve {} ({})",
            self.path.display(),
            self.cause
        )
    }
}

impl std::error::Error for ResolutionFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

pub(super) fn failed_path(error: &std::io::Error) -> Option<&Path> {
    error
        .get_ref()?
        .downcast_ref::<ResolutionFailure>()
        .map(|failure| failure.path.as_path())
}

/// `path` (absolute) resolved the way the kernel walks it: component by component, each existing prefix resolved
/// through the file system (symlinks followed) BEFORE a later `..` applies, so `own/link/../x` with `link` pointing
/// into another copy resolves next to the link's target, as the kernel would (folding `..`
/// lexically first gave `own/x`). A dangling symlink leaf is resolved through its target so creating that target
/// cannot escape the guard. Once an ordinary prefix does not exist the rest is folded lexically (the kernel would
/// fail there unless a separate operation first creates that prefix).
///
/// # Errors
/// A filesystem read fails for a reason other than a missing path, or a symlink chain exceeds 40 hops.
pub fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    resolve_with_hops(path, &mut 0)
        .map(|real| normalize_prefix(&real))
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                ResolutionFailure {
                    path: path.to_owned(),
                    cause: error,
                },
            )
        })
}

// Keep the canonical Windows prefix even when no component exists yet (or the path is just a root).
// Using verbatim form preserves long paths and names that cannot safely be expressed in DOS form.
#[cfg(windows)]
pub(super) fn normalize_prefix(path: &Path) -> PathBuf {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return path.to_path_buf();
    };
    let Some(root) = normalized_prefix(prefix.kind()) else {
        return path.to_path_buf();
    };
    let mut out = PathBuf::from(root);
    out.extend(parts);
    out
}

// Separate prefix spelling from the host parser so Windows prefix cases can also be checked on Linux.
#[cfg(any(windows, test))]
pub(super) fn normalized_prefix(prefix: std::path::Prefix<'_>) -> Option<std::ffi::OsString> {
    use std::path::Prefix;

    match prefix {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            Some(format!(r"\\?\{}:", char::from(drive.to_ascii_uppercase())).into())
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            let mut root = std::ffi::OsString::from(r"\\?\UNC\");
            root.push(server.to_ascii_lowercase());
            root.push(r"\");
            root.push(share.to_ascii_lowercase());
            Some(root)
        }
        _ => None,
    }
}

#[cfg(not(windows))]
pub(super) fn normalize_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
}

// Verbatim Windows components recognize only backslashes; Vibe can send forward slashes even with that prefix.
#[cfg(windows)]
pub(super) fn normalize_separators(path: &Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let units: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == u16::from(b'/') {
                u16::from(b'\\')
            } else {
                unit
            }
        })
        .collect();
    PathBuf::from(std::ffi::OsString::from_wide(&units))
}

#[cfg(not(windows))]
pub(super) fn normalize_separators(path: &Path) -> PathBuf {
    path.to_path_buf()
}

// canonicalize returns verbatim paths, but an ordinary input must keep Win32 filename semantics while probing
// later components (for example, `worktrees.` can name `worktrees`). Normalize identity only after walking it.
#[cfg(windows)]
fn path_for_probe(path: &Path, input: &Path) -> PathBuf {
    use std::path::Prefix;

    if matches!(input.components().next(), Some(Component::Prefix(prefix)) if prefix.kind().is_verbatim())
    {
        return path.to_path_buf();
    }
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return path.to_path_buf();
    };
    let root = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => std::ffi::OsString::from(format!("{}:", char::from(drive))),
        Prefix::VerbatimUNC(server, share) => {
            let mut root = std::ffi::OsString::from(r"\\");
            root.push(server);
            root.push(r"\");
            root.push(share);
            root
        }
        _ => return path.to_path_buf(),
    };
    let mut out = PathBuf::from(root);
    out.extend(parts);
    out
}

#[cfg(not(windows))]
fn path_for_probe(path: &Path, _input: &Path) -> PathBuf {
    path.to_path_buf()
}

// PathBuf::join on a verbatim base folds `..` before a junction can be followed. Keep the raw spelling until resolve.
pub(super) fn join_for_resolution(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute()
        || matches!(path.components().next(), Some(Component::Prefix(_)))
        || base.as_os_str().is_empty()
    {
        return path.to_path_buf();
    }
    let mut out = if path.has_root() {
        match base.components().next() {
            Some(Component::Prefix(prefix)) => prefix.as_os_str().to_os_string(),
            _ => std::ffi::OsString::new(),
        }
    } else {
        let mut out = base.as_os_str().to_os_string();
        out.push(std::path::MAIN_SEPARATOR_STR);
        out
    };
    out.push(path.as_os_str());
    PathBuf::from(out)
}

// A junction is a reparse point even on Windows versions that do not classify it as a symbolic link.
// If canonicalization fails, read_link must resolve its target or fail closed rather than treating it as absent.
fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_type().is_symlink() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn resolve_with_hops(path: &Path, hops: &mut usize) -> std::io::Result<PathBuf> {
    use std::io::ErrorKind::{InvalidFilename, NotFound};

    let path = normalize_separators(path);
    let mut out = PathBuf::new();
    let mut exists = true;
    for part in path.components() {
        // Verbatim components retain literal dot names; the guard still walks them in kernel order.
        let part = match part {
            Component::Normal(name) if name == "." => Component::CurDir,
            Component::Normal(name) if name == ".." => Component::ParentDir,
            part => part,
        };
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
                exists = path_for_probe(&out, &path).try_exists()?;
            }
            Component::Normal(name) => {
                out.push(name);
                if exists {
                    let probe = path_for_probe(&out, &path);
                    match std::fs::canonicalize(&probe) {
                        Ok(real) => out = real,
                        Err(error) => match std::fs::symlink_metadata(&probe) {
                            Ok(meta) if is_link(&meta) => {
                                *hops += 1;
                                if *hops > 40 {
                                    return Err(std::io::Error::other("too many symbolic links"));
                                }
                                let target = std::fs::read_link(&probe)?;
                                let target = if target.is_absolute() {
                                    target
                                } else {
                                    join_for_resolution(
                                        out.parent().unwrap_or(Path::new("")),
                                        &target,
                                    )
                                };
                                out = resolve_with_hops(&target, hops)?;
                                exists = path_for_probe(&out, &path).try_exists()?;
                            }
                            Ok(_) if matches!(error.kind(), NotFound | InvalidFilename) => {
                                exists = false;
                            }
                            Err(meta_error)
                                if matches!(meta_error.kind(), NotFound | InvalidFilename)
                                    && matches!(error.kind(), NotFound | InvalidFilename) =>
                            {
                                exists = false;
                            }
                            Err(meta_error) => return Err(meta_error),
                            _ => return Err(error),
                        },
                    }
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                // A drive prefix alone (`C:`) is not the absolute path being resolved. It can fail or
                // refer to the drive's current directory. Only probe after the first normal component.
                out.push(part.as_os_str());
            }
        }
    }
    Ok(out)
}
