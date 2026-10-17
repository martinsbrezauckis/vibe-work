//! Bounded, no-git discovery for the project browser. Depends on std; called by the core.
//! Never follows symbolic links or walks external copies. External pointers are metadata, never action targets.

use std::collections::{BTreeSet, VecDeque};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

/// Discovery budgets, shared by directory entries and worktree metadata.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Root is depth zero.
    pub depth: usize,
    /// Maximum entries inspected.
    pub entries: usize,
    /// Wall-clock discovery budget.
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
    };
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
        small_text(&git_dir.join("commondir")).and_then(|s| safe_path(&git_dir, Path::new(&s)))
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
/// Find repositories without invoking git, with a hard total budget including linked-copy records.
#[must_use]
pub fn discover(project: &Path, limits: Limits) -> Discovery {
    let mut out = Discovery::default();
    let Some(root) = safe_path(Path::new(""), project) else {
        out.truncated = true;
        return out;
    };
    let Ok(metadata) = root.symlink_metadata() else {
        out.truncated = true;
        return out;
    };
    let start = Instant::now();
    let mut remaining = limits.entries;
    let mut stack = VecDeque::from([(root.clone(), 0)]);
    let mut seen = BTreeSet::new();
    while let Some((dir, depth)) = stack.pop_front() {
        if remaining == 0 || start.elapsed() >= limits.time || out.nodes.len() >= 128 {
            out.truncated = true;
            break;
        }
        if !seen.insert(dir.clone()) {
            continue;
        }
        // A queued child may have been replaced since read_dir reported its file type.
        if !dir.starts_with(&root)
            || safe_path(Path::new(""), &dir).as_deref() != Some(dir.as_path())
            || !dir.is_dir()
            || !same_device(&metadata, &dir)
        {
            out.truncated = true;
            continue;
        }
        out.directories.push(dir.clone());
        if let Some(node) = classify(&dir, &root) {
            out.nodes.push(node);
        }
        let Some(entries) = directory_entries(&dir) else {
            out.truncated = true;
            continue;
        };
        for entry in entries {
            if remaining == 0 || start.elapsed() >= limits.time {
                out.truncated = true;
                break;
            }
            remaining -= 1;
            let Ok(entry) = entry else {
                out.truncated = true;
                continue;
            };
            if skipped(&entry.file_name().unwrap_or_default().to_string_lossy()) {
                continue;
            }
            let Ok(kind) = entry.symlink_metadata().map(|m| m.file_type()) else {
                out.truncated = true;
                continue;
            };
            if kind.is_dir() && same_device(&metadata, &entry) {
                if depth < limits.depth {
                    stack.push_back((entry, depth + 1));
                } else {
                    out.truncated = true;
                }
            }
        }
    }
    linked_copies(&root, &mut out, &mut remaining, start, limits);
    out.nodes.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn linked_copies(
    root: &Path,
    out: &mut Discovery,
    remaining: &mut usize,
    start: Instant,
    limits: Limits,
) {
    let common: BTreeSet<_> = out
        .nodes
        .iter()
        .filter(|n| n.restricted.is_none())
        .filter_map(|n| n.common_dir.clone())
        .collect();
    let mut paths: BTreeSet<_> = out.nodes.iter().map(|n| n.path.clone()).collect();
    for common in common {
        let Some(records) = safe_path(&common, Path::new("worktrees")) else {
            continue;
        };
        let Some(entries) = directory_entries(&records) else {
            continue;
        };
        for entry in entries {
            if *remaining == 0 || start.elapsed() >= limits.time || out.nodes.len() >= 128 {
                out.truncated = true;
                break;
            }
            *remaining -= 1;
            let Ok(entry) = entry else {
                continue;
            };
            let Some(record) = safe_path(Path::new(""), &entry) else {
                continue;
            };
            let Some(marker) =
                small_text(&record.join("gitdir")).and_then(|s| safe_path(&record, Path::new(&s)))
            else {
                continue;
            };
            if marker.file_name().is_none_or(|n| n != ".git") {
                continue;
            }
            let Some(folder) = marker.parent() else {
                continue;
            };
            let outside = !folder.starts_with(root);
            let node = if outside {
                // The worktree record is the authority for an external location. Never read its marker.
                Node {
                    path: folder.into(),
                    git_dir: Some(record.clone()),
                    common_dir: Some(common.clone()),
                    kind: "copy".into(),
                    outside: true,
                    restricted: None,
                }
            } else {
                let Some(node) = classify(folder, root) else {
                    continue;
                };
                if node.git_dir.as_ref() != Some(&record)
                    || node.common_dir.as_ref() != Some(&common)
                {
                    continue;
                }
                node
            };
            if !paths.insert(folder.into()) {
                continue;
            }
            out.nodes.push(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Limits, discover, revalidate};
    use std::fs;
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::time::Instant;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let tick = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("vw-git-browser-{}-{tick}", std::process::id()));
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
