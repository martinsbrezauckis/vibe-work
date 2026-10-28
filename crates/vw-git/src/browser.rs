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

#[cfg(test)]
mod tests {
    use super::{Limits, discover, discover_complete, revalidate};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[cfg(unix)]
    use std::time::Instant;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            // Tests run in parallel and some clocks tick in microseconds, so a counter keeps names apart.
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let tick = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            // Discovery never opens a path through a symbolic link, and the macOS temporary folder lies below
            // the `/var` link, so Unix fixtures live below the resolved temporary folder.
            let base = std::env::temp_dir();
            let base = if cfg!(unix) {
                base.canonicalize().unwrap()
            } else {
                base
            };
            let path = base.join(format!("vw-git-browser-{}-{tick}-{n}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn dir(&self, path: &str) -> PathBuf {
            let dir = self.0.join(path);
            fs::create_dir_all(&dir).unwrap();
            dir
        }
        fn write(&self, path: &str, value: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, value).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn scan(project: &Path) -> super::Discovery {
        discover(
            project,
            Limits {
                depth: 4,
                entries: 1_000,
                time: Duration::from_secs(5),
            },
        )
    }

    #[test]
    fn nested_repositories_and_linked_copies_group_by_common_dir() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        fixture.dir("project/main/.git/worktrees/task");
        fixture.dir("project/nested/other/.git");
        fixture.dir("project/copy");
        fixture.write("project/main/.git/worktrees/task/commondir", "../..\n");
        fixture.write(
            "project/main/.git/worktrees/task/gitdir",
            "../../../../copy/.git\n",
        );
        fixture.write("project/copy/.git", "gitdir: ../main/.git/worktrees/task\n");
        let found = scan(&project);
        assert!(!found.truncated, "{found:?}");
        assert_eq!(found.nodes.len(), 3, "{found:?}");
        assert!(found.nodes.iter().any(|n| n.path == project.join("copy")
            && n.kind == "copy"
            && n.common_dir.as_deref() == Some(project.join("main/.git").as_path())));
    }

    #[test]
    fn expired_slices_preserve_nested_repositories_and_linked_records() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        fixture.dir("project/.git/worktrees/outside");
        fixture.dir("project/.git/worktrees/inside");
        fixture.dir("project/nested/.git");
        fixture.dir("project/inside");
        fixture.dir("outside");
        fixture.write("outside/.git", "not read outside scope\n");
        fixture.write(
            "project/.git/worktrees/outside/gitdir",
            "../../../../outside/.git\n",
        );
        fixture.write(
            "project/.git/worktrees/inside/gitdir",
            "../../../inside/.git\n",
        );
        fixture.write("project/.git/worktrees/inside/commondir", "../..\n");
        fixture.write("project/inside/.git", "gitdir: ../.git/worktrees/inside\n");
        let limits = Limits {
            time: Duration::ZERO,
            ..Limits::default()
        };
        let partial = discover(&project, limits);
        assert!(partial.truncated);
        assert_eq!(partial.nodes.len(), 1);
        assert_eq!(partial.nodes[0].path, project);

        let complete = discover_complete(&project, limits);
        let uninterrupted = scan(&project);
        assert!(!complete.truncated, "{complete:?}");
        assert_eq!(complete.nodes.len(), 4, "{complete:?}");
        assert_eq!(complete.directories, uninterrupted.directories);
        for (sliced, whole) in complete.nodes.iter().zip(&uninterrupted.nodes) {
            assert_eq!(sliced.path, whole.path);
            assert_eq!(sliced.git_dir, whole.git_dir);
            assert_eq!(sliced.common_dir, whole.common_dir);
            assert_eq!(sliced.kind, whole.kind);
            assert_eq!(sliced.outside, whole.outside);
            assert_eq!(sliced.restricted, whole.restricted);
            if !sliced.outside {
                assert!(revalidate(sliced, &project));
            }
        }
        assert!(complete.directories.iter().all(|p| p.starts_with(&project)));
    }

    #[test]
    fn resumed_discovery_shares_entry_and_depth_limits() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        for name in ["one", "two", "three"] {
            fixture.dir(&format!("project/.git/worktrees/{name}"));
            fixture.dir(name);
            fixture.write(&format!("{name}/.git"), "not read\n");
            fixture.write(
                &format!("project/.git/worktrees/{name}/gitdir"),
                &format!("../../../../{name}/.git\n"),
            );
        }
        let found = discover_complete(
            &project,
            Limits {
                entries: 2,
                time: Duration::ZERO,
                ..Limits::default()
            },
        );
        assert!(found.truncated);
        assert_eq!(
            found.nodes.len(),
            2,
            "root entry and one worktree record: {found:?}"
        );

        fixture.dir("project/nested/.git");
        let found = discover_complete(
            &project,
            Limits {
                depth: 0,
                time: Duration::ZERO,
                ..Limits::default()
            },
        );
        assert!(found.truncated);
        assert_eq!(found.nodes.len(), 4);
        assert_eq!(found.directories, vec![project]);
    }

    #[test]
    fn resumed_discovery_keeps_the_node_limit() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        for i in 0..130 {
            fixture.dir(&format!("project/repo-{i}/.git"));
        }
        let found = discover_complete(
            &project,
            Limits {
                time: Duration::ZERO,
                ..Limits::default()
            },
        );
        assert!(found.truncated);
        assert_eq!(found.nodes.len(), 128);
    }

    #[test]
    fn outside_copy_is_metadata_only_and_invalid_pointers_are_restricted() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        fixture.dir("project/main/.git/worktrees/outside");
        fixture.dir("outside");
        fixture.write(
            "project/main/.git/worktrees/outside/gitdir",
            "../../../../../outside/.git\n",
        );
        fixture.write("outside/.git", "gitdir: /bad/path\n");
        fixture.dir("project/bad");
        fixture.write("project/bad/.git", "gitdir: ../../outside/.git\n");
        fixture.dir("project/imposter");
        fixture.write("project/imposter/.git", "gitdir: ../main/.git\n");
        let found = scan(&project);
        let external = found
            .nodes
            .iter()
            .find(|n| n.path == fixture.0.join("outside"))
            .unwrap();
        assert!(
            external.outside && external.restricted.is_none(),
            "{external:?}"
        );
        assert_eq!(external.kind, "copy");
        let bad = found
            .nodes
            .iter()
            .find(|n| n.path == project.join("bad"))
            .unwrap();
        assert!(bad.restricted.is_some());
        assert!(bad.git_dir.is_none());
        let imposter = found
            .nodes
            .iter()
            .find(|n| n.path == project.join("imposter"))
            .unwrap();
        assert!(imposter.restricted.is_some());
    }

    #[test]
    fn budgets_caches_and_symlink_loops_cannot_expand_discovery() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        fixture.dir("project/node_modules/hidden/.git");
        fixture.dir("project/target/hidden/.git");
        for i in 0..30 {
            fixture.dir(&format!("project/big/{i}"));
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&project, project.join("loop")).unwrap();
        }
        let found = discover(
            &project,
            Limits {
                depth: 4,
                entries: 10,
                time: Duration::from_secs(5),
            },
        );
        assert!(found.truncated);
        assert!(found.nodes.is_empty());
        assert!(
            found
                .directories
                .iter()
                .all(|p| !p.to_string_lossy().contains("node_modules")
                    && !p.to_string_lossy().contains("target")
                    && !p.ends_with("loop"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn fifo_metadata_never_blocks_discovery() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        let record = fixture.dir("project/main/.git/worktrees/task");
        let copy = fixture.dir("project/copy");
        fixture.write("project/copy/.git", "gitdir: ../main/.git/worktrees/task\n");
        for path in [record.join("gitdir"), record.join("commondir")] {
            let status = std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
        }
        let start = Instant::now();
        let found = scan(&project);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "FIFO blocked discovery"
        );
        let node = found.nodes.iter().find(|node| node.path == copy).unwrap();
        assert!(node.restricted.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn pointer_open_refuses_linked_parent_leaf_and_fifo() {
        let fixture = Fixture::new();
        let real = fixture.dir("real");
        fixture.write("real/pointer", "gitdir: here\n");
        std::os::unix::fs::symlink(&real, fixture.0.join("linked-parent")).unwrap();
        std::os::unix::fs::symlink(real.join("pointer"), fixture.0.join("linked-leaf")).unwrap();
        let fifo = fixture.0.join("pipe");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert!(super::open_regular(&real.join("pointer")).is_some());
        assert!(super::open_regular(&fixture.0.join("linked-parent/pointer")).is_none());
        assert!(super::open_regular(&fixture.0.join("linked-leaf")).is_none());
        assert!(super::directory_entries(&real).is_some());
        assert!(super::directory_entries(&fixture.0.join("linked-parent")).is_none());
        let start = Instant::now();
        assert!(super::open_regular(&fifo).is_none());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlink_cannot_extend_project_walk_or_worktree_records() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        fixture.dir("project/main/.git");
        fixture.dir("outside/repo/.git");
        std::os::unix::fs::symlink(fixture.0.join("outside"), project.join("linked")).unwrap();
        std::os::unix::fs::symlink(
            fixture.0.join("outside"),
            project.join("main/.git/worktrees"),
        )
        .unwrap();
        let found = scan(&project);
        assert_eq!(found.nodes.len(), 1, "{found:?}");
        assert!(found.directories.iter().all(|dir| !dir.ends_with("linked")));
        assert!(super::directory_entries(&project.join("main/.git/worktrees")).is_none());
    }

    #[test]
    fn revalidation_rejects_pointer_replacement_and_malformed_lines() {
        let fixture = Fixture::new();
        let project = fixture.dir("project");
        fixture.dir("project/main/.git/worktrees/task");
        fixture.dir("project/copy");
        fixture.write("project/main/.git/worktrees/task/commondir", "../..\n");
        fixture.write(
            "project/main/.git/worktrees/task/gitdir",
            "../../../../copy/.git\n",
        );
        fixture.write("project/copy/.git", "gitdir: ../main/.git/worktrees/task\n");
        let found = scan(&project);
        let copy = found
            .nodes
            .iter()
            .find(|node| node.path == project.join("copy"))
            .unwrap();
        assert!(revalidate(copy, &project));
        fixture.write("project/copy/.git", "gitdir: \n");
        assert!(!revalidate(copy, &project));
        fixture.write(
            "project/copy/.git",
            "gitdir: ../main/.git/worktrees/task\nextra\n",
        );
        assert!(!revalidate(copy, &project));
    }
}

#[cfg(test)]
#[cfg(windows)]
#[path = "browser_windows_tests.rs"]
mod windows_tests;
