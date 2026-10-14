//! A stat cache for the working tree: what we know about each file (size, modification and
//! change time, inode, mode) and the blob id last computed for it, so a refresh hashes only files that
//! changed (what git's index does for `git add`, which we may not run: it would run the repository's clean
//! filters).
//!
//! Depends on: `serde_json`, std. Called by: `worktree`, `baseline`.
//! Invariants (review integrity: the agent being reviewed must not be able to hide an edit):
//! - A hit needs size, modification time, **change time** and inode (+ device) to match. A normal user
//!   cannot set the change time on Unix (`touch -r` and `utimes` reset it to now), so restoring the old
//!   modification time after a same-size edit is still seen.
//! - Racy entries (changed within `RACY` of when they were hashed) are hashed again.
//! - Platforms where the change time is not available through std (Windows) never trust the cache:
//!   [`trusted`] is false there and every refresh hashes every file.
//! - Callers bypass the cache before every keep/revert and on a periodic full pass (`Baseline::scan`).

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Changes closer than this to the hash time are not trusted to the cache.
const RACY: Duration = Duration::from_secs(2);

/// True where the cache can be trusted (the change time and inode are readable).
#[must_use]
pub(crate) fn trusted() -> bool {
    cfg!(unix)
}

/// The facts a cache hit must match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stat {
    pub(crate) len: u64,
    pub(crate) mtime_ns: u128,
    pub(crate) ctime_ns: u128,
    pub(crate) ino: u64,
    pub(crate) dev: u64,
}

impl Stat {
    /// From `lstat` metadata.
    #[cfg(unix)]
    pub(crate) fn of(meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        let ctime = u128::try_from(meta.ctime()).unwrap_or(0) * 1_000_000_000
            + u128::try_from(meta.ctime_nsec()).unwrap_or(0);
        Self {
            len: meta.len(),
            mtime_ns: meta.modified().map_or(0, ns),
            ctime_ns: ctime,
            ino: meta.ino(),
            dev: meta.dev(),
        }
    }

    /// From metadata (no change time or inode through std here: the cache is not used, see [`trusted`]).
    #[cfg(not(unix))]
    pub(crate) fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            len: meta.len(),
            mtime_ns: meta.modified().map_or(0, ns),
            ctime_ns: 0,
            ino: 0,
            dev: 0,
        }
    }

    fn newest(&self) -> u128 {
        self.mtime_ns.max(self.ctime_ns)
    }
}

/// One cached file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Cached {
    pub(crate) stat: Stat,
    pub(crate) mode: String,
    pub(crate) id: String,
    /// When the id was computed (ns since the epoch).
    pub(crate) hashed_ns: u128,
}

/// Shown path -> cached facts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct StatCache {
    pub(crate) files: HashMap<String, Cached>,
    #[serde(skip)]
    pub(crate) dirty: bool,
}

/// Nanoseconds since the epoch of `time` (0 before the epoch).
pub(crate) fn ns(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

impl StatCache {
    /// Reads `path` (empty when missing, unreadable or of an older shape).
    pub(crate) fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Writes `path` when something changed.
    pub(crate) fn save(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(self)
            && std::fs::write(path, bytes).is_ok()
        {
            self.dirty = false;
        }
    }

    /// The id for `key` when every fact matches and the entry is not racy.
    pub(crate) fn hit(&self, key: &str, stat: &Stat, mode: &str) -> Option<&str> {
        let c = self.files.get(key)?;
        let racy = c.hashed_ns.saturating_sub(stat.newest()) < RACY.as_nanos();
        (c.stat == *stat && c.mode == mode && !racy).then_some(c.id.as_str())
    }

    /// Records a freshly computed id.
    pub(crate) fn put(&mut self, key: String, stat: Stat, mode: String, id: String) {
        let hashed_ns = ns(SystemTime::now());
        self.files.insert(
            key,
            Cached {
                stat,
                mode,
                id,
                hashed_ns,
            },
        );
        self.dirty = true;
    }

    /// Drops paths that no longer exist.
    pub(crate) fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        let before = self.files.len();
        self.files.retain(|k, _| keep(k));
        if self.files.len() != before {
            self.dirty = true;
        }
    }
}
