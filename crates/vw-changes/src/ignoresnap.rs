//! The ignore rules as they were when the review started (integrity check): the agent being
//! reviewed can write `.gitignore` files, `.git/info/exclude` and `.git/config` (`core.excludesFile`), so the
//! rules git applies *now* must not decide which new files the review lists. We keep a copy of the
//! baseline's rules in a private shadow repository and ask `git check-ignore` there.
//!
//! Depends on: `git`, `objects`, `paths`, `store`, `repo`, `error`. Called by: `baseline` (take, load),
//! `worktree` (new files hidden by later rules), vw-core-api (the watcher's ignore list).
//! Invariants:
//! - The shadow (`<session>/ignore/`) holds only ignore files: every baseline `.gitignore` at its path, the
//!   repository's `info/exclude` and the effective `core.excludesFile` as they were at baseline. It is our
//!   own repository in our data folder; nothing is written into the user's.
//! - `check-ignore` there reads patterns only (no index, no file content).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::ChangesError;
use crate::git::{Call, Index};
use crate::objects::{Entries, read_blobs};
use crate::paths;
use crate::repo::Repo;
use crate::store::SessionDir;

/// The baseline's ignore rules for one session.
#[derive(Debug, Clone)]
pub(crate) struct IgnoreSnapshot {
    root: PathBuf,
    global: PathBuf,
}

fn is_gitignore(path: &[u8]) -> bool {
    path == b".gitignore" || path.ends_with(b"/.gitignore")
}

impl IgnoreSnapshot {
    fn at(session: &SessionDir) -> Self {
        Self {
            root: session.path().join("ignore"),
            global: session.path().join("ignore-global"),
        }
    }

    /// The stored snapshot, when there is one.
    pub(crate) fn load(session: &SessionDir) -> Option<Self> {
        let snapshot = Self::at(session);
        (snapshot.root.join(".git").is_dir() && snapshot.global.is_file()).then_some(snapshot)
    }

    /// Records the rules of `base` (the baseline entries) and the repository's current exclude files.
    pub(crate) fn take(
        repo: &Repo,
        session: &SessionDir,
        base: &Entries,
    ) -> Result<Self, ChangesError> {
        let snapshot = Self::at(session);
        if snapshot.root.exists() {
            std::fs::remove_dir_all(&snapshot.root)
                .map_err(|e| ChangesError::io("remove", &snapshot.root, e))?;
        }
        std::fs::create_dir_all(&snapshot.root)
            .map_err(|e| ChangesError::io("create", &snapshot.root, e))?;
        let scratch = session.scratch_index();
        Call::new(&snapshot.root, Index::Ours(&scratch))
            .args(["init", "-q"])
            .run()?;
        let ignores: Vec<(&Vec<u8>, &str)> = base
            .iter()
            .filter(|(p, e)| is_gitignore(p) && e.mode.starts_with("100"))
            .map(|(p, e)| (p, e.id.as_str()))
            .collect();
        let ids: Vec<&str> = ignores.iter().map(|(_, id)| *id).collect();
        let blobs = read_blobs(repo, &session.current_index(), &ids)?;
        for (path, id) in ignores {
            let Some(bytes) = blobs.get(id) else { continue };
            let target = paths::inside_bytes(&snapshot.root, path)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ChangesError::io("create", parent, e))?;
            }
            std::fs::write(&target, bytes).map_err(|e| ChangesError::io("write", &target, e))?;
        }
        if let Some(exclude) = read_exclude(repo)? {
            let info = snapshot.root.join(".git/info");
            std::fs::create_dir_all(&info).map_err(|e| ChangesError::io("create", &info, e))?;
            let path = info.join("exclude");
            std::fs::write(&path, exclude).map_err(|e| ChangesError::io("write", &path, e))?;
        }
        let global = read_global(repo).unwrap_or_default();
        std::fs::write(&snapshot.global, global)
            .map_err(|e| ChangesError::io("write", &snapshot.global, e))?;
        Ok(snapshot)
    }

    /// Which of the folders `dirs` (repository-relative bytes, no trailing slash) the baseline's rules
    /// ignore as a whole. The folder is created (empty) in the shadow first, so git sees a directory exactly
    /// as its own walker does: asking about `dir/` as text would match `dir/*` against the folder itself,
    /// which git's walker never does (it then walks in and finds `!dir/keep.txt`).
    pub(crate) fn ignored_dirs(&self, dirs: &[Vec<u8>]) -> Result<HashSet<Vec<u8>>, ChangesError> {
        for dir in dirs {
            let full = paths::inside_bytes(&self.root, dir)?;
            std::fs::create_dir_all(&full).map_err(|e| ChangesError::io("create", &full, e))?;
        }
        self.ignored(dirs)
    }

    /// Which of the files `paths` (repository-relative bytes) the baseline's rules ignore.
    pub(crate) fn ignored(&self, paths: &[Vec<u8>]) -> Result<HashSet<Vec<u8>>, ChangesError> {
        if paths.is_empty() {
            return Ok(HashSet::new());
        }
        let mut input = Vec::new();
        for path in paths {
            input.extend_from_slice(path);
            input.push(0);
        }
        let mut global = std::ffi::OsString::from("core.excludesFile=");
        global.push(&self.global);
        let scratch = self.root.join(".git/vw-scratch.idx");
        let out = Call::new(&self.root, Index::Ours(&scratch))
            .args([std::ffi::OsString::from("-c"), global])
            .args(["check-ignore", "--no-index", "--stdin", "-z"])
            .stdin(&input)
            .output()?;
        // Exit 1 = nothing ignored; anything above is an error.
        if out.code.is_none_or(|c| c > 1) {
            return Err(ChangesError::Git {
                command: "check-ignore".into(),
                message: crate::error::clip(&out.stderr, 400),
            });
        }
        Ok(out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(<[u8]>::to_vec)
            .collect())
    }
}

/// The repository's `info/exclude` (shared by linked worktrees), when it exists.
fn read_exclude(repo: &Repo) -> Result<Option<Vec<u8>>, ChangesError> {
    let out = Call::new(&repo.root, Index::User)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .run()?;
    let text = String::from_utf8_lossy(&out).trim().to_owned();
    let path = Path::new(&text);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo.root.join(path)
    };
    Ok(std::fs::read(path).ok())
}

/// The effective `core.excludesFile` (repository and user config, or git's default location).
fn read_global(repo: &Repo) -> Option<Vec<u8>> {
    let configured = Call::new(&repo.root, Index::User)
        .args(["config", "--path", "--get", "core.excludesFile"])
        .run()
        .ok()
        .map(|v| String::from_utf8_lossy(&v).trim().to_owned())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let path = configured.or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(|x| PathBuf::from(x).join("git/ignore"))
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/git/ignore"))
            })
    })?;
    std::fs::read(path).ok()
}
