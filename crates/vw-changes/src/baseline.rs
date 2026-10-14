//! A session's review baseline: a tree object of the working tree when the session started,
//! kept as an index file and a tree id in our app data.
//!
//! Depends on: `git`, `objects`, `statcache`, `repo`, `store`, `worktree`, `error`. Called by: `diff`,
//! `review`, vw-core-api.
//! Invariants:
//! - Taking, loading and moving a baseline writes only our files and loose objects in the repository; no
//!   ref, no index, no HEAD, no stash, no config. The working tree is read by us, never by git
//!   (`worktree::scan`), so no repository filter or hook runs.
//! - Every blob the baseline needs that `HEAD` does not reach is copied into `blobs/`, so after a
//!   `git gc --prune=now` [`Baseline::ensure_objects`] can write them back and rebuild the exact same tree.
//!   If even that fails (history rewritten and pruned), the baseline is reported lost, never guessed.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use crate::error::ChangesError;
use crate::git::{Call, Index};
use crate::ignoresnap::IgnoreSnapshot;
use crate::objects::write_tree;
use crate::objects::{Entries, read_index, write_index};
use crate::repo::{Repo, head_tree};
use crate::statcache::StatCache;
use crate::store::{BaselineState, STATE_VERSION, SessionDir, now_ms};
use crate::worktree::{self, LargeFile, Scan};

/// How often a refresh ignores the stat cache and reads every file (review integrity: a cache fooled in a
/// way we did not foresee is corrected within this time).
pub const FULL_PASS: std::time::Duration = std::time::Duration::from_secs(60);

/// The result of opening a session's stored baseline.
#[derive(Debug)]
pub enum Loaded {
    /// No baseline was ever taken for this session.
    None,
    /// The intact baseline.
    Ready(Box<Baseline>),
    /// A baseline was taken but cannot be trusted any more (why, for the user). Show changes against the
    /// last commit and take a new baseline only when the user asks ([`diff_against_head`]).
    Lost(String),
}

/// The changes of the working tree against `HEAD`, for a session whose baseline was lost. Builds a separate
/// baseline from `HEAD` in `dir/against-head/` (the session's own baseline files are not touched) and
/// returns it with its first diff; refresh it again with [`crate::refresh`].
///
/// # Errors
/// git and filesystem failures.
pub fn against_head(repo: Repo, dir: &SessionDir) -> Result<Baseline, ChangesError> {
    Baseline::from_head(repo, dir.child("against-head")?)
}

/// [`against_head`] and its diff, in one call.
///
/// # Errors
/// git and filesystem failures.
pub fn diff_against_head(repo: Repo, dir: &SessionDir) -> Result<crate::Diff, ChangesError> {
    crate::refresh(&against_head(repo, dir)?)
}

/// A session's baseline.
#[derive(Debug)]
pub struct Baseline {
    repo: Repo,
    dir: SessionDir,
    state: BaselineState,
    /// Untracked files left out for their size when the baseline was taken.
    large: Vec<LargeFile>,
    /// The baseline as index entries (what `base.idx` holds).
    entries: Entries,
    /// The working tree's stat cache (`stat.json`).
    cache: Mutex<StatCache>,
    /// When every file was last read and hashed regardless of the cache.
    last_full: Mutex<std::time::Instant>,
}

impl Baseline {
    /// Takes a new baseline of the working tree now (replaces any earlier one of this session).
    ///
    /// # Errors
    /// git or filesystem failures; nothing of the old baseline is kept on failure.
    pub fn take(repo: Repo, dir: SessionDir) -> Result<Self, ChangesError> {
        Self::build(repo, dir, true)
    }

    /// A baseline equal to `HEAD`'s tree (a fresh Vibe worktree is a clean checkout; its working tree may
    /// already hold the agent's first edits when we notice the folder change).
    ///
    /// # Errors
    /// git or filesystem failures.
    pub fn from_head(repo: Repo, dir: SessionDir) -> Result<Self, ChangesError> {
        Self::build(repo, dir, false)
    }

    fn build(mut repo: Repo, dir: SessionDir, with_worktree: bool) -> Result<Self, ChangesError> {
        let base = dir.base_index();
        let current = dir.current_index();
        for stale in [&base, &current, &dir.path().join("stat.json")] {
            remove_if_present(stale)?;
        }
        let head = head_tree(&repo, &base)?;
        seed(&repo, &base, head.as_deref())?;
        let seeded = read_index(&repo, &base)?;
        let mut cache = StatCache::default();
        let (entries, large) = if with_worktree {
            let scan = worktree::scan(&repo, &seeded, &base, &mut cache, false, None)?;
            write_index(&repo, &base, &scan.entries)?;
            if scan.tracked_only {
                repo.tracked_only = true;
            }
            (scan.entries, scan.large)
        } else {
            (seeded, Vec::new())
        };
        let tree = write_tree(&repo, &base)?;
        let now = now_ms();
        let state = BaselineState {
            version: STATE_VERSION,
            repo_root: repo.root.clone(),
            scope: repo.scope.clone(),
            tracked_only: repo.tracked_only,
            tree,
            taken_at_ms: now,
            updated_at_ms: now,
        };
        IgnoreSnapshot::take(&repo, &dir, &entries)?;
        let baseline = Self {
            repo,
            dir,
            state,
            large,
            entries,
            cache: Mutex::new(cache),
            last_full: Mutex::new(std::time::Instant::now()),
        };
        baseline.save_blobs()?;
        std::fs::copy(&base, &current).map_err(|e| ChangesError::io("copy", &base, e))?;
        baseline.persist()?;
        Ok(baseline)
    }

    /// The stored baseline of this session, when there is one and it is intact (see [`Baseline::open`]).
    #[must_use]
    pub fn load(repo: Repo, dir: SessionDir) -> Option<Self> {
        match Self::open(repo, dir) {
            Loaded::Ready(baseline) => Some(*baseline),
            Loaded::None | Loaded::Lost(_) => None,
        }
    }

    /// Opens the stored baseline. Never fails open (integrity check): a stored baseline that is
    /// unreadable, for another repository or scope, or whose index no longer builds the recorded tree (an
    /// edited `base.idx` or `state.json`, objects that cannot be repaired) is `Lost`, never `None`, so the
    /// caller cannot take a new baseline without the user asking for it.
    #[must_use]
    pub fn open(mut repo: Repo, dir: SessionDir) -> Loaded {
        if !dir.has_state() {
            return Loaded::None;
        }
        let Some(state) = dir.read_state() else {
            return Loaded::Lost("the stored starting point cannot be read".into());
        };
        if state.repo_root != repo.root || state.scope != repo.scope {
            return Loaded::Lost("the stored starting point belongs to another folder".into());
        }
        let entries = match read_index(&repo, &dir.base_index()) {
            Ok(entries) => entries,
            Err(error) => {
                return Loaded::Lost(format!("the stored starting point is damaged ({error})"));
            }
        };
        let large = std::fs::read(dir.path().join("large.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // What the review covers was decided at baseline; a later `status.showUntrackedFiles=no` written
        // by the agent into `.git/config` must not hide new files.
        repo.tracked_only = state.tracked_only;
        let cache = StatCache::load(&dir.path().join("stat.json"));
        let baseline = Self {
            repo,
            dir,
            state,
            large,
            entries,
            cache: Mutex::new(cache),
            last_full: Mutex::new(std::time::Instant::now()),
        };
        if let Err(error) = baseline.ensure_objects() {
            return Loaded::Lost(error.to_string());
        }
        match write_tree(&baseline.repo, &baseline.dir.base_index()) {
            Ok(tree) if tree == baseline.state.tree => {}
            Ok(_) => {
                return Loaded::Lost(
                    "the stored starting point was changed after it was taken".into(),
                );
            }
            Err(error) => {
                return Loaded::Lost(format!("the stored starting point is damaged ({error})"));
            }
        }
        if IgnoreSnapshot::load(&baseline.dir).is_none()
            && let Err(error) =
                IgnoreSnapshot::take(&baseline.repo, &baseline.dir, &baseline.entries)
        {
            return Loaded::Lost(format!("the stored ignore rules are missing ({error})"));
        }
        Loaded::Ready(Box::new(baseline))
    }

    /// The repository.
    #[must_use]
    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    /// The session's data folder.
    #[must_use]
    pub fn dir(&self) -> &SessionDir {
        &self.dir
    }

    /// The persisted facts (tree id, times).
    #[must_use]
    pub fn state(&self) -> &BaselineState {
        &self.state
    }

    /// Untracked files that were too large to record when the baseline was taken.
    #[must_use]
    pub fn large(&self) -> &[LargeFile] {
        &self.large
    }

    /// The baseline's index entries.
    pub(crate) fn entries(&self) -> Entries {
        self.entries.clone()
    }

    /// Reads the working tree against `base` (the stat cache is updated and saved). With `trust` false, or
    /// when the last full pass is older than [`FULL_PASS`], every file is read and hashed again.
    pub(crate) fn scan(&self, base: &Entries, trust: bool) -> Result<Scan, ChangesError> {
        let full = {
            let mut last = self
                .last_full
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let due = !trust || last.elapsed() >= FULL_PASS;
            if due {
                *last = std::time::Instant::now();
            }
            due
        };
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let rules = IgnoreSnapshot::load(&self.dir);
        let scan = worktree::scan(
            &self.repo,
            base,
            &self.dir.base_index(),
            &mut cache,
            !full,
            rules.as_ref(),
        )?;
        cache.save(&self.dir.path().join("stat.json"));
        Ok(scan)
    }

    /// Drops the stat cache (after a forced gc its ids may name pruned blobs): every file is stored again.
    pub(crate) fn forget_stat_cache(&self) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        *cache = StatCache::default();
        let _ = std::fs::remove_file(self.dir.path().join("stat.json"));
    }

    /// Replaces the list of large untracked files the baseline accepts as they are (a keep of such a
    /// file); persisted by the next [`Baseline::commit_index`].
    pub fn set_large(&mut self, large: Vec<crate::worktree::LargeFile>) {
        self.large = large;
    }

    /// Moves the baseline to the tree now in `base.idx` (after a keep), saving the blobs it needs.
    ///
    /// # Errors
    /// git or filesystem failures.
    pub fn commit_index(&mut self) -> Result<(), ChangesError> {
        self.state.tree = write_tree(&self.repo, &self.dir.base_index())?;
        self.entries = read_index(&self.repo, &self.dir.base_index())?;
        self.state.updated_at_ms = now_ms();
        self.save_blobs()?;
        self.persist()
    }

    /// Makes sure the baseline tree exists in the repository; after a forced gc writes the saved blobs back
    /// and rebuilds the tree from `base.idx`. Returns true when it had to repair.
    ///
    /// # Errors
    /// `BaselineLost` when the rebuilt tree differs or cannot be written (blobs `HEAD` reached are gone too).
    pub fn ensure_objects(&self) -> Result<bool, ChangesError> {
        if self.has_object(&self.state.tree)? {
            return Ok(false);
        }
        let mut list = Vec::new();
        let entries = std::fs::read_dir(self.dir.blobs())
            .map_err(|e| ChangesError::io("read", self.dir.blobs(), e))?;
        for entry in entries.flatten() {
            list.extend_from_slice(entry.path().to_string_lossy().as_bytes());
            list.push(b'\n');
        }
        if !list.is_empty() {
            Call::new(&self.repo.root, Index::Ours(&self.dir.base_index()))
                .args(["hash-object", "-w", "--no-filters", "--stdin-paths"])
                .stdin(&list)
                .run()?;
        }
        let rebuilt = write_tree(&self.repo, &self.dir.base_index())
            .map_err(|e| ChangesError::BaselineLost(e.to_string()))?;
        if rebuilt != self.state.tree {
            return Err(ChangesError::BaselineLost(format!(
                "rebuilt tree {rebuilt} differs from {}",
                self.state.tree
            )));
        }
        Ok(true)
    }

    fn has_object(&self, id: &str) -> Result<bool, ChangesError> {
        Ok(
            Call::new(&self.repo.root, Index::Ours(&self.dir.base_index()))
                .args(["cat-file", "-e", id])
                .output()?
                .ok(),
        )
    }

    fn persist(&self) -> Result<(), ChangesError> {
        let path = self.dir.path().join("large.json");
        let bytes = serde_json::to_vec(&self.large)
            .map_err(|e| ChangesError::Parse(format!("large.json: {e}")))?;
        std::fs::write(&path, bytes).map_err(|e| ChangesError::io("write", &path, e))?;
        self.dir.write_state(&self.state)
    }

    /// Copies every baseline blob `HEAD` does not reach into `blobs/` and drops copies no longer needed.
    fn save_blobs(&self) -> Result<(), ChangesError> {
        let base = self.dir.base_index();
        let needed = blobs_outside_head(&self.repo, &base)?;
        let folder = self.dir.blobs();
        let have: HashSet<String> = std::fs::read_dir(&folder)
            .map_err(|e| ChangesError::io("read", &folder, e))?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let missing: Vec<&String> = needed.iter().filter(|id| !have.contains(*id)).collect();
        if !missing.is_empty() {
            let mut request = Vec::new();
            for id in &missing {
                request.extend_from_slice(id.as_bytes());
                request.push(b'\n');
            }
            let out = Call::new(&self.repo.root, Index::Ours(&base))
                .args(["cat-file", "--batch"])
                .stdin(&request)
                .run()?;
            for (id, bytes) in crate::objects::parse_batch(&out)? {
                let path = folder.join(&id);
                std::fs::write(&path, bytes).map_err(|e| ChangesError::io("save", &path, e))?;
            }
        }
        for stale in have.difference(&needed) {
            let _ = std::fs::remove_file(folder.join(stale));
        }
        Ok(())
    }
}

fn remove_if_present(path: &Path) -> Result<(), ChangesError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ChangesError::io("remove", path, e)),
    }
}

/// Fills `index` with `HEAD`'s tree, or empties it on an unborn branch.
fn seed(repo: &Repo, index: &Path, head: Option<&str>) -> Result<(), ChangesError> {
    let call = Call::new(&repo.root, Index::Ours(index));
    match head {
        Some(tree) => call.args(["read-tree", tree]).run()?,
        None => call.args(["read-tree", "--empty"]).run()?,
    };
    Ok(())
}

/// Blob ids in `index` that differ from `HEAD` (all of them on an unborn branch); gitlinks excluded.
fn blobs_outside_head(repo: &Repo, index: &Path) -> Result<HashSet<String>, ChangesError> {
    let Some(head) = head_tree(repo, index)? else {
        return all_blobs(repo, index);
    };
    let out = Call::new(&repo.root, Index::Ours(index))
        .args([
            "diff-index",
            "--cached",
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-renames",
            &head,
        ])
        .run()?;
    let mut needed = HashSet::new();
    // Records: ":<mode a> <mode b> <id a> <id b> <status>\0<path>\0".
    let mut fields = out.split(|b| *b == 0);
    while let Some(meta) = fields.next() {
        if meta.is_empty() {
            continue;
        }
        let _path = fields.next();
        let text = String::from_utf8_lossy(meta);
        let parts: Vec<&str> = text.trim_start_matches(':').split(' ').collect();
        if parts.len() < 5 {
            return Err(ChangesError::Parse(format!("diff-index record {text:?}")));
        }
        let (mode, id) = (parts[1], parts[3]);
        if mode != "000000" && mode != "160000" && id.bytes().any(|b| b != b'0') {
            needed.insert(id.to_owned());
        }
    }
    Ok(needed)
}

/// Every blob id in `index` (an unborn branch: nothing is reachable), gitlinks excluded.
fn all_blobs(repo: &Repo, index: &Path) -> Result<HashSet<String>, ChangesError> {
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["ls-files", "--stage", "-z"])
        .run()?;
    // Records: "<mode> <id> <stage>\t<path>\0".
    Ok(out
        .split(|b| *b == 0)
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let meta = text.split('\t').next()?.to_owned();
            let mut parts = meta.split(' ');
            let mode = parts.next()?;
            let id = parts.next()?;
            (mode != "160000").then(|| id.to_owned())
        })
        .collect())
}
