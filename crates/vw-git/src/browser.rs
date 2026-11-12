//! Bounded, no-git discovery for the project browser. Depends on std; called by the core.
//! Never follows symbolic links or walks external copies. External pointers are metadata, never action targets.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Discovery budgets, shared by directory entries and worktree metadata.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Root is depth zero.
    pub depth: usize,
    /// Maximum entries inspected.
    pub entries: usize,
    /// Wall-clock budget per discovery slice; `discover` returns after the first slice.
    pub time: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: 4,
            entries: 10_000,
            time: Duration::from_millis(250),
        }
    }
}
/// A discovered repository or linked copy.
#[derive(Debug, Clone)]
pub struct Node {
    /// Absolute folder.
    pub path: PathBuf,
    /// Git state folder when safely resolved.
    pub git_dir: Option<PathBuf>,
    /// Common state folder, used to group copies.
    pub common_dir: Option<PathBuf>,
    /// repository, copy, or submodule.
    pub kind: String,
    /// External copies are metadata only.
    pub outside: bool,
    /// Why status cannot safely run.
    pub restricted: Option<String>,
}
/// Discovery output, including the directories eligible for nonrecursive watches.
#[derive(Debug, Default)]
pub struct Discovery {
    /// Found nodes.
    pub nodes: Vec<Node>,
    /// Bounded directories walked, never external.
    pub directories: Vec<PathBuf>,
    /// Budget exhaustion, permission failures, or skipped symbolic links make discovery incomplete.
    pub truncated: bool,
}
/// Cache and generated directory names never traversed or watched.
#[must_use]
pub fn skipped(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | "out"
            | ".cache"
            | "__pycache__"
            | ".venv"
            | "venv"
            | ".next"
            | ".turbo"
    )
}
/// Lexically normalize a pointer, then reject symbolic links in every existing component.
///
/// On Windows the drive or share part (`C:`, `\\?\C:`, `\\server\share`) is not a folder entry of its own and is
/// not checked alone: the first entry checked is the drive's root folder (`C:\`, `\\?\C:\`). Checking the bare
/// part asked about the drive's current folder for `C:` and failed outright for the `\\?\C:` form that
/// `canonicalize` gives ("Incorrect function"), so a canonical folder was never readable.
#[must_use]
pub fn safe_path(base: &Path, value: &Path) -> Option<PathBuf> {
    let joined = base.join(value);
    let mut clean = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                if !clean.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    let mut check = PathBuf::new();
    for part in clean.components() {
        check.push(part.as_os_str());
        // The drive or share part is checked together with the root folder that follows it (see above).
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        let meta = check.symlink_metadata().ok()?;
        if meta.file_type().is_symlink() {
            return None;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return None;
            }
        }
    }
    Some(clean)
}
#[cfg(unix)]
fn open_dir_nofollow(path: &Path) -> Option<std::os::fd::OwnedFd> {
    use rustix::fs::{CWD, Mode, OFlags, openat};
    if !path.is_absolute() {
        return None;
    }
    let mut dir = openat(
        CWD,
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    for part in path.components() {
        match part {
            Component::Normal(name) => {
                dir = openat(
                    &dir,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .ok()?;
            }
            Component::RootDir => {}
            _ => return None,
        }
    }
    Some(dir)
}
#[cfg(unix)]
fn open_regular(path: &Path) -> Option<std::fs::File> {
    use rustix::fs::{Mode, OFlags, openat};
    let dir = open_dir_nofollow(path.parent()?)?;
    let leaf = path.file_name()?;
    let fd = openat(
        &dir,
        leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let file = std::fs::File::from(fd);
    file.metadata().ok()?.is_file().then_some(file)
}
#[cfg(unix)]
fn directory_entries(path: &Path) -> Option<Box<dyn Iterator<Item = Result<PathBuf, ()>>>> {
    use std::os::unix::ffi::OsStrExt;
    let dir = rustix::fs::Dir::new(open_dir_nofollow(path)?).ok()?;
    let base = path.to_path_buf();
    Some(Box::new(dir.filter_map(move |entry| match entry {
        Ok(entry) => {
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                None
            } else {
                Some(Ok(base.join(std::ffi::OsStr::from_bytes(name))))
            }
        }
        Err(_) => Some(Err(())),
    })))
}
#[cfg(not(unix))]
fn directory_entries(path: &Path) -> Option<Box<dyn Iterator<Item = Result<PathBuf, ()>>>> {
    Some(Box::new(std::fs::read_dir(path).ok()?.map(|entry| {
        entry.map(|entry| entry.path()).map_err(|_| ())
    })))
}
#[cfg(not(unix))]
fn open_regular(path: &Path) -> Option<std::fs::File> {
    if !path.symlink_metadata().ok()?.file_type().is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    file.metadata().ok()?.is_file().then_some(file)
}
fn small_text(path: &Path) -> Option<String> {
    safe_path(Path::new(""), path)?;
    let file = open_regular(path)?;
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let value = String::from_utf8(bytes).ok()?;
    Some(value.trim_end_matches(['\r', '\n']).to_owned())
}
/// `path`, a location read from git's own pointer files (a `gitdir:` line, a `commondir` or a worktree record), in
/// the project's spelling when it lies in `project`; otherwise `path` unchanged, so the caller treats it as outside.
///
/// Git writes absolute pointers in its own spelling of a folder: on Windows `C:/...` with long names and the on-disk
/// case, whatever spelling the folder had when git ran. The project folder may be written with a short name or in
/// another case. Both sides have passed [`safe_path`] (no symbolic link or junction in any component), so when the
/// lexical comparison fails their canonical forms differ from them only in spelling: those are compared, and the
/// inside part is joined to `project` and checked by [`safe_path`] again. A path that cannot be canonicalized, lies
/// outside, or meets a link on the second check stays as git wrote it.
fn in_project_spelling(path: PathBuf, project: &Path) -> PathBuf {
    if path.starts_with(project) {
        return path;
    }
    let (Ok(real), Ok(root)) = (path.canonicalize(), project.canonicalize()) else {
        return path;
    };
    match real.strip_prefix(&root) {
        Ok(rest) => safe_path(project, rest).unwrap_or(path),
        Err(_) => path,
    }
}
fn classify(folder: &Path, project: &Path) -> Option<Node> {
    let marker = folder.join(".git");
    let metadata = marker.symlink_metadata().ok()?;
    let mut node = Node {
        path: folder.into(),
        git_dir: None,
        common_dir: None,
        kind: "repository".into(),
        outside: false,
        restricted: None,
    };
    let target = if metadata.is_dir() {
        safe_path(folder, Path::new(".git"))
    } else if metadata.is_file() {
        small_text(&marker)
            .and_then(|s| s.strip_prefix("gitdir: ").map(str::to_owned))
            .and_then(|s| safe_path(folder, Path::new(&s)))
    } else {
        None
    }
    .map(|git_dir| in_project_spelling(git_dir, project));
    let Some(git_dir) = target else {
        node.restricted =
            Some("The repository pointer is missing, invalid, or a symbolic link.".into());
        return Some(node);
    };
    if !git_dir.starts_with(project) {
        node.restricted =
            Some("Repository data points outside this project; only its location is shown.".into());
        return Some(node);
    }
    if !git_dir.is_dir() {
        node.restricted = Some("The repository pointer does not name a git directory.".into());
        return Some(node);
    }
    let common = if git_dir.join("commondir").symlink_metadata().is_ok() {
        small_text(&git_dir.join("commondir"))
            .and_then(|s| safe_path(&git_dir, Path::new(&s)))
            .map(|common| in_project_spelling(common, project))
    } else {
        Some(git_dir.clone())
    };
    node.kind = if common.as_ref().is_some_and(|p| *p != git_dir) {
        "copy"
    } else if metadata.is_file() {
        "submodule"
    } else {
        "repository"
    }
    .into();
    if common
        .as_ref()
        .is_none_or(|p| !p.starts_with(project) || !p.is_dir())
    {
        node.restricted =
            Some("Repository data points outside this project; only its location is shown.".into());
    } else if metadata.is_file() {
        let linked = common.as_ref().is_some_and(|p| {
            git_dir
                .parent()
                .is_some_and(|parent| parent == p.join("worktrees"))
                && small_text(&git_dir.join("gitdir"))
                    .and_then(|s| safe_path(&git_dir, Path::new(&s)))
                    .map(|marker| in_project_spelling(marker, project))
                    .as_deref()
                    == Some(marker.as_path())
        });
        let submodule = git_dir.components().any(|c| c.as_os_str() == "modules");
        if !(linked || node.kind == "submodule" && submodule) {
            node.restricted =
                Some("The repository pointer is not a linked copy or submodule.".into());
        }
    }
    node.git_dir = Some(git_dir);
    node.common_dir = common;
    Some(node)
}
/// Recheck a discovered, in-project node before a later git status call.
/// This reduces stale-pointer use but cannot make a path-based check and git spawn atomic.
#[must_use]
pub fn revalidate(node: &Node, project: &Path) -> bool {
    !node.outside
        && node.restricted.is_none()
        && node.path.starts_with(project)
        && safe_path(Path::new(""), &node.path).as_deref() == Some(node.path.as_path())
        && classify(&node.path, project).is_some_and(|current| {
            current.restricted.is_none()
                && current.kind == node.kind
                && current.git_dir == node.git_dir
                && current.common_dir == node.common_dir
        })
}
#[cfg(unix)]
fn same_device(root: &std::fs::Metadata, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    path.symlink_metadata().is_ok_and(|m| m.dev() == root.dev())
}
#[cfg(not(unix))]
fn same_device(_: &std::fs::Metadata, _: &Path) -> bool {
    true
}
#[path = "browser_walk.rs"]
mod walk;
pub use walk::{discover, discover_complete};
