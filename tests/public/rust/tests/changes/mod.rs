//! Throwaway repositories support Changes integration tests.

#![allow(dead_code, unreachable_pub)]

use vw_public_tests::test_paths;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use vw_changes::{Baseline, Diff, FileChange, SessionDir};

static SEQ: AtomicU32 = AtomicU32::new(0);

/// A temporary repository plus a changes data folder; removed on drop.
pub struct TempRepo {
    pub base: PathBuf,
    pub root: PathBuf,
    pub data: PathBuf,
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

impl TempRepo {
    /// `git init` with a local identity and the given files committed (empty list: unborn branch); its
    /// `data` folder is prepared as a test data root (git calls need its hooks folder).
    pub fn with_files(files: &[(&str, &str)]) -> Self {
        let repo = Self::unprepared(files);
        vw_changes::prepare_root(&repo.data).unwrap();
        repo
    }

    /// As [`TempRepo::with_files`] without preparing `data` (`vw-changes` git calls refuse until some data
    /// root is prepared in the process).
    pub fn unprepared(files: &[(&str, &str)]) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let base = test_paths::root().join(format!(
            "vw-changes-{}-{}-{n}",
            std::process::id(),
            vw_changes::now_ms()
        ));
        let root = base.join("repo");
        let data = base.join("data");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let repo = Self {
            base,
            root: root.canonicalize().unwrap(),
            data,
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "core.autocrlf", "false"]);
        if !files.is_empty() {
            for (path, text) in files {
                repo.write(path, text);
            }
            repo.git(&["add", "-A"]);
            repo.git(&["commit", "-q", "-m", "init"]);
        }
        repo
    }

    /// Runs git in the repository (test setup only) and returns stdout; panics on failure.
    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn write(&self, rel: &str, text: &str) {
        self.write_bytes(rel, text.as_bytes());
    }

    pub fn write_bytes(&self, rel: &str, bytes: &[u8]) {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        String::from_utf8(std::fs::read(self.path(rel)).unwrap()).unwrap()
    }

    pub fn read_bytes(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.path(rel)).unwrap()
    }

    /// Discovers the repository and takes a baseline for `session`.
    pub fn baseline(&self, session: &str) -> Baseline {
        let repo = vw_changes::discover(&self.root)
            .unwrap()
            .expect("a repository");
        let dir = SessionDir::open(&self.data, session).unwrap();
        Baseline::take(repo, dir).unwrap()
    }

    /// Loads the stored baseline of `session` again (as after an app restart).
    pub fn reload(&self, session: &str) -> Baseline {
        let repo = vw_changes::discover(&self.root)
            .unwrap()
            .expect("a repository");
        let dir = SessionDir::open(&self.data, session).unwrap();
        Baseline::load(repo, dir).expect("stored baseline")
    }

    /// Everything git keeps as state outside `objects/`: every file under `.git` (bytes), plus the views a
    /// user would look at.
    pub fn state(&self) -> RepoState {
        let git_dir = self.root.join(".git");
        let mut files = BTreeMap::new();
        collect(&git_dir, &git_dir, &mut files);
        RepoState {
            files,
            status: self.git(&["status", "--porcelain=v1", "--untracked-files=all"]),
            refs: self.git(&["for-each-ref"]),
            stash: self.git(&["stash", "list"]),
        }
    }
}

fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "objects" || rel.starts_with("objects/") {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            out.insert(rel, std::fs::read(&path).unwrap_or_default());
        }
    }
}

/// See [`TempRepo::state`].
#[derive(Debug, PartialEq, Eq)]
pub struct RepoState {
    pub files: BTreeMap<String, Vec<u8>>,
    pub status: String,
    pub refs: String,
    pub stash: String,
}

/// The file at `path` in `diff`, or a panic naming what is there.
pub fn file<'a>(diff: &'a Diff, path: &str) -> &'a FileChange {
    diff.file(path).unwrap_or_else(|| {
        panic!(
            "{path} not in the diff: {:?}",
            diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
        )
    })
}

/// `n` numbered lines `line 1` .. `line n`, each ending with `\n`.
pub fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}
