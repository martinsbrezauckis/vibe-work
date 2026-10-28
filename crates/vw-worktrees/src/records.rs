//! What Vibe Work remembers about each session copy it made: one JSON file per session in a user-only
//! folder of the app data (`<data>/worktrees/records/<session id>.json`).
//!
//! A record says where the copy came from (repository, base branch and commit), which branch we created,
//! the folder name Vibe got, and what the user decided at the end (kept or removed). It is what the session
//! header shows, what a removal checks the branch rule against ("delete the branch only when we created it
//! and it has no new commits"), and what the startup list of orphans is built from.
//!
//! Depends on: serde, vw-platform (`ensure_private_dir`), vw-changes (`valid_session_id`, `now_ms`), `error`.
//! Called by: vw-core-api.
//! Invariants:
//! - Writes are atomic (temp file + rename); failed starts can retain a separate recovery record and
//!   an in-memory copy shared by cloned handles. A record that cannot be parsed is skipped (logged), never
//!   guessed.
//! - A file name is a valid session id (no path tricks).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::error::WorktreeError;

mod recovery;
mod rename;

// A read can finalize a rename. Serialize it with updates so readers do not reuse the same
// temporary file or overwrite metadata that changed while the branch was being reconciled.
static RECORDS_LOCK: Mutex<()> = Mutex::new(());

fn lock_records() -> MutexGuard<'static, ()> {
    RECORDS_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the user decided about a copy at the end of its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    /// "Keep the worktree": never asked about again.
    Kept,
    /// Removed through Vibe Work.
    Removed,
}

/// One session copy. Serde ignores unknown fields, including old `sessionArchived` flags;
/// those flags no longer grant archive or removal authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// Vibe's session id.
    pub session_id: String,
    /// Root of the repository's main working tree (where the session was started).
    pub repo_root: PathBuf,
    /// The base the user picked (branch name, or the commit id of a detached checkout).
    pub base_name: String,
    /// The commit the branch was created at.
    pub base_commit: String,
    /// The session's branch.
    pub branch: String,
    /// The worktree folder name given to Vibe.
    pub folder: String,
    /// Vibe Work created the branch (always true for copies made here; kept explicit for the rule).
    pub created_branch: bool,
    /// When the copy was asked for (ms since the epoch).
    pub created_at_ms: u64,
    /// The user's decision at the end, if any.
    #[serde(default)]
    pub decision: Option<Decision>,
    /// The session was deleted (its copy may be an orphan).
    #[serde(default)]
    pub session_deleted: bool,
}

/// The record folder.
#[derive(Debug, Clone)]
pub struct Records {
    dir: PathBuf,
    recovery: Arc<Mutex<BTreeMap<String, Record>>>,
}

impl Records {
    /// Opens (and creates, user-only) the record folder.
    ///
    /// # Errors
    /// `Io` when the folder cannot be created or restricted.
    pub fn open(dir: &Path) -> Result<Self, WorktreeError> {
        vw_platform::ensure_private_dir(dir).map_err(|e| {
            WorktreeError::io(
                "prepare the private folder",
                dir,
                std::io::Error::other(e.to_string()),
            )
        })?;
        Ok(Self {
            dir: dir.to_path_buf(),
            recovery: Arc::default(),
        })
    }

    fn file(&self, session_id: &str) -> Option<PathBuf> {
        vw_changes::valid_session_id(session_id)
            .then(|| self.dir.join(format!("{session_id}.json")))
    }

    /// The record of `session_id`, if any.
    #[must_use]
    pub fn get(&self, session_id: &str) -> Option<Record> {
        let _guard = lock_records();
        self.get_unlocked(session_id)
    }

    fn get_unlocked(&self, session_id: &str) -> Option<Record> {
        if let Some(record) = self.memory_recovery(session_id) {
            return self.follow_rename(record).ok();
        }
        let recovery = self.recovery_file(session_id)?;
        let bytes = if recovery.exists() {
            std::fs::read(recovery).ok()?
        } else {
            std::fs::read(self.file(session_id)?).ok()?
        };
        match serde_json::from_slice::<Record>(&bytes) {
            Ok(record) if record.session_id == session_id => self.follow_rename(record).ok(),
            Ok(_) | Err(_) => {
                tracing::warn!(session_id, "worktrees: unreadable record skipped");
                None
            }
        }
    }

    /// Every readable record.
    #[must_use]
    pub fn all(&self) -> Vec<Record> {
        let mut ids = self.recovery_ids();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            ids.extend(entries.flatten().filter_map(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .strip_suffix(".json")
                    .map(str::to_owned)
            }));
        }
        ids.sort();
        ids.dedup();
        let mut out: Vec<Record> = ids.iter().filter_map(|id| self.get(id)).collect();
        out.sort_by_key(|r| r.created_at_ms);
        out
    }

    /// Every record for authorization checks. Unlike [`Self::all`], any unreadable or malformed record is an error:
    /// silently dropping an active owner's record could let another session use its copy.
    ///
    /// # Errors
    /// A record directory entry cannot be read, or a named record cannot be read or parsed.
    pub fn all_for_guard(&self) -> Result<Vec<Record>, WorktreeError> {
        let _guard = lock_records();
        let entries = std::fs::read_dir(&self.dir)
            .map_err(|e| WorktreeError::io("read records", &self.dir, e))?;
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| WorktreeError::io("read record entry", &self.dir, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if !vw_changes::valid_session_id(id) {
                continue;
            }
            let path = entry.path();
            let bytes =
                std::fs::read(&path).map_err(|e| WorktreeError::io("read record", &path, e))?;
            let record: Record = serde_json::from_slice(&bytes)
                .map_err(|e| WorktreeError::io("parse record", &path, std::io::Error::other(e)))?;
            if record.session_id != id {
                return Err(WorktreeError::InvalidName(format!(
                    "record {} names a different session",
                    path.display()
                )));
            }
            out.push(self.follow_rename(record)?);
        }
        self.add_recoveries_for_guard(&mut out)?;
        out.sort_by_key(|r| r.created_at_ms);
        Ok(out)
    }

    /// Writes `record` (replacing an earlier one of the same session).
    ///
    /// # Errors
    /// `InvalidName` for a session id that cannot name a file; `Io` on write failures.
    pub fn put(&self, record: &Record) -> Result<(), WorktreeError> {
        let _guard = lock_records();
        self.put_unlocked(record)
    }

    fn put_unlocked(&self, record: &Record) -> Result<(), WorktreeError> {
        if self.is_recovery(&record.session_id) {
            return self.recover_unlocked(record);
        }
        let Some(path) = self.file(&record.session_id) else {
            return Err(WorktreeError::InvalidName(
                "the session id cannot name a record file".into(),
            ));
        };
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|e| WorktreeError::io("encode", &path, std::io::Error::other(e)))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| WorktreeError::io("write", &tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| WorktreeError::io("replace", &path, e))
    }

    /// Changes the record of `session_id` with `change` and writes it. `Ok(None)` when there is none.
    ///
    /// # Errors
    /// As [`Records::put`].
    pub fn update(
        &self,
        session_id: &str,
        change: impl FnOnce(&mut Record),
    ) -> Result<Option<Record>, WorktreeError> {
        let _guard = lock_records();
        let Some(mut record) = self.get_unlocked(session_id) else {
            return Ok(None);
        };
        change(&mut record);
        self.put_unlocked(&record)?;
        Ok(Some(record))
    }

    /// Moves the record of session `old` to `new` (Vibe handed the conversation to a new session id: the copy
    /// is now `new`'s, and its Keep/Remove choice is offered for `new`). The new file is written before
    /// the old one is removed. `Ok(false)` when `old` has no record or `new` already has one (kept as it is).
    ///
    /// # Errors
    /// As [`Records::put`].
    pub fn hand_off(&self, old: &str, new: &str) -> Result<bool, WorktreeError> {
        let Some(mut record) = self.get(old) else {
            return Ok(false);
        };
        if self.get(new).is_some() {
            return Ok(false);
        }
        new.clone_into(&mut record.session_id);
        // Ending the old conversation grants no removal permission for its new owner.
        record.session_deleted = false;
        self.put(&record)?;
        if self.is_recovery(old) {
            self.remove_recovery(old)?;
        }
        if let Some(path) = self.file(old)
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(WorktreeError::io("remove", &path, error));
        }
        Ok(true)
    }
}
