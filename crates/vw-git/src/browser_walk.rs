//! Resumable project discovery for the Git browser. Depends on browser path checks and std.
//! The core waits for all slices; directory, entry and node limits still bound the complete walk.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{
    Discovery, Limits, Node, classify, directory_entries, in_project_spelling, safe_path,
    same_device, skipped, small_text,
};

type Entries = Box<dyn Iterator<Item = Result<PathBuf, ()>>>;

enum Work {
    Directory(PathBuf, usize),
    Entries(Entries, usize),
    Common(PathBuf),
    Records(PathBuf, Entries),
}

struct Walk {
    root: PathBuf,
    metadata: std::fs::Metadata,
    limits: Limits,
    remaining: usize,
    work: VecDeque<Work>,
    seen: BTreeSet<PathBuf>,
    paths: BTreeSet<PathBuf>,
    linked: bool,
    out: Discovery,
}

impl Walk {
    fn new(project: &Path, limits: Limits) -> Option<Self> {
        let root = safe_path(Path::new(""), project)?;
        let metadata = root.symlink_metadata().ok()?;
        Some(Self {
            work: VecDeque::from([Work::Directory(root.clone(), 0)]),
            root,
            metadata,
            limits,
            remaining: limits.entries,
            seen: BTreeSet::new(),
            paths: BTreeSet::new(),
            linked: false,
            out: Discovery::default(),
        })
    }

    // Every slice makes progress, even if the worker was descheduled before its first entry.
    fn advance(&mut self) -> bool {
        let start = Instant::now();
        loop {
            if self.work.is_empty() && !self.linked {
                self.linked = true;
                let common: BTreeSet<_> = self
                    .out
                    .nodes
                    .iter()
                    .filter(|n| n.restricted.is_none())
                    .filter_map(|n| n.common_dir.clone())
                    .collect();
                for common in common {
                    self.work.push_back(Work::Common(common));
                }
            }
            if self.work.is_empty() {
                return true;
            }
            if self.remaining == 0 || self.out.nodes.len() >= 128 {
                self.out.truncated = true;
                self.work.clear();
                self.linked = true;
                return true;
            }
            if let Some(work) = self.work.pop_front() {
                match work {
                    Work::Directory(dir, depth) => self.directory(dir, depth),
                    Work::Entries(mut entries, depth) => {
                        if let Some(entry) = entries.next() {
                            self.work.push_front(Work::Entries(entries, depth));
                            self.entry(entry, depth);
                        }
                    }
                    Work::Common(common) => {
                        if let Some(records) = safe_path(&common, Path::new("worktrees"))
                            && let Some(entries) = directory_entries(&records)
                        {
                            self.work.push_front(Work::Records(common, entries));
                        }
                    }
                    Work::Records(common, mut entries) => {
                        if let Some(entry) = entries.next() {
                            self.record(&common, entry);
                            self.work.push_front(Work::Records(common, entries));
                        }
                    }
                }
            }
            if start.elapsed() >= self.limits.time {
                return false;
            }
        }
    }

    fn directory(&mut self, dir: PathBuf, depth: usize) {
        if !self.seen.insert(dir.clone()) {
            return;
        }
        // Recheck queued folders so a replacement cannot extend the project walk.
        if !dir.starts_with(&self.root)
            || safe_path(Path::new(""), &dir).as_deref() != Some(dir.as_path())
            || !dir.is_dir()
            || !same_device(&self.metadata, &dir)
        {
            self.out.truncated = true;
            return;
        }
        if let Some(node) = classify(&dir, &self.root) {
            self.paths.insert(node.path.clone());
            self.out.nodes.push(node);
        }
        if let Some(entries) = directory_entries(&dir) {
            self.work.push_front(Work::Entries(entries, depth));
        } else {
            self.out.truncated = true;
        }
        self.out.directories.push(dir);
    }

    fn entry(&mut self, entry: Result<PathBuf, ()>, depth: usize) {
        self.remaining -= 1;
        let Ok(entry) = entry else {
            self.out.truncated = true;
            return;
        };
        if skipped(&entry.file_name().unwrap_or_default().to_string_lossy()) {
            return;
        }
        let Ok(kind) = entry.symlink_metadata().map(|m| m.file_type()) else {
            self.out.truncated = true;
            return;
        };
        if kind.is_dir() && same_device(&self.metadata, &entry) {
            if depth < self.limits.depth {
                self.work.push_back(Work::Directory(entry, depth + 1));
            } else {
                self.out.truncated = true;
            }
        }
    }

    fn record(&mut self, common: &Path, entry: Result<PathBuf, ()>) {
        self.remaining -= 1;
        let Ok(entry) = entry else { return };
        let Some(record) = safe_path(Path::new(""), &entry) else {
            return;
        };
        let Some(marker) = small_text(&record.join("gitdir"))
            .and_then(|s| safe_path(&record, Path::new(&s)))
            .map(|marker| in_project_spelling(marker, &self.root))
        else {
            return;
        };
        if marker.file_name().is_none_or(|n| n != ".git") {
            return;
        }
        let Some(folder) = marker.parent() else {
            return;
        };
        let node = if folder.starts_with(&self.root) {
            let Some(node) = classify(folder, &self.root) else {
                return;
            };
            if node.git_dir.as_ref() != Some(&record) || node.common_dir.as_deref() != Some(common)
            {
                return;
            }
            node
        } else {
            // External copies expose their in-project record, never their marker or status.
            Node {
                path: folder.into(),
                git_dir: Some(record),
                common_dir: Some(common.into()),
                kind: "copy".into(),
                outside: true,
                restricted: None,
            }
        };
        if self.paths.insert(folder.into()) {
            self.out.nodes.push(node);
        }
    }

    fn finish(mut self) -> Discovery {
        self.out.nodes.sort_by(|a, b| a.path.cmp(&b.path));
        self.out
    }
}

/// Find repositories without invoking git, returning an incomplete snapshot when any budget expires.
#[must_use]
pub fn discover(project: &Path, limits: Limits) -> Discovery {
    run(project, limits, false)
}

/// Finish bounded discovery on a worker, yielding between timed slices without losing pending entries.
/// Callers keep the browser loading until this returns; entry, depth and node limits apply across all slices.
#[must_use]
pub fn discover_complete(project: &Path, limits: Limits) -> Discovery {
    run(project, limits, true)
}

fn run(project: &Path, limits: Limits, complete: bool) -> Discovery {
    let Some(mut walk) = Walk::new(project, limits) else {
        return Discovery {
            truncated: true,
            ..Discovery::default()
        };
    };
    while !walk.advance() {
        if !complete {
            walk.out.truncated = true;
            break;
        }
        std::thread::yield_now();
    }
    walk.finish()
}
