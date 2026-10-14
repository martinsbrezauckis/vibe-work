//! Where a session's review data lives in our app data folder, and its small state file.
//!
//! Layout: `<changes dir>/sessions/<session id>/` with `state.json` (baseline tree, repository root, scope,
//! when it was taken), `base.idx` (the baseline as an index: paths, modes, blob ids), `cur.idx` (the working
//! tree as last seen, with git's stat cache), `blobs/<id>` (copies of baseline blobs that `HEAD` does not
//! reach, to rebuild the baseline after a `git gc --prune=now`); `<changes dir>/offers.json` (folders where
//! the user said "don't ask again" to the git offer).
//! Depends on: `serde_json`, vw-platform (`ensure_private_dir`), `error`. Called by: `baseline`, `diff`,
//! `review`, `plain`, vw-core-api.
//! Invariants:
//! - The changes dir is user-only (0700 / user-only DACL inherited by its children); it is created once.
//! - A session id becomes a folder name only if it is 1-128 characters of `[A-Za-z0-9._-]` and not `.`/`..`.
//! - `state.json` is replaced atomically (written next to it, then renamed): a crash leaves the old or the
//!   new state, never half of one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ChangesError;

/// Version of `state.json`.
pub const STATE_VERSION: u32 = 1;

/// The persisted baseline facts of one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineState {
    /// Format version (`STATE_VERSION`).
    pub version: u32,
    /// The repository root the baseline belongs to.
    pub repo_root: PathBuf,
    /// The reviewed subtree (`Repo::scope`).
    pub scope: String,
    /// Untracked files are not part of the review (`Repo::tracked_only`).
    pub tracked_only: bool,
    /// The baseline tree id.
    pub tree: String,
    /// When the review started (ms since the Unix epoch).
    pub taken_at_ms: u64,
    /// When the baseline last moved (a keep), ms since the Unix epoch.
    pub updated_at_ms: u64,
}

/// One session's folder in the changes dir.
#[derive(Debug, Clone)]
pub struct SessionDir {
    dir: PathBuf,
}

/// True when `id` may become a folder name.
#[must_use]
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Makes the changes dir user-only (once per process is enough; cheap on Unix, `icacls` on Windows).
///
/// # Errors
/// `Io` when the folder cannot be created or restricted.
pub fn prepare_root(changes_dir: &Path) -> Result<(), ChangesError> {
    vw_platform::ensure_private_dir(changes_dir).map_err(|e| {
        ChangesError::io(
            "prepare the private folder",
            changes_dir,
            std::io::Error::other(e.to_string()),
        )
    })?;
    // The empty folder every git call uses as `core.hooksPath` (inside the user-only folder, so nobody
    // else can put a hook there).
    // Checked, never just created: a symlink or a folder that already holds files is refused (`hooks`).
    crate::hooks::register(&changes_dir.join("no-hooks"))
}

impl SessionDir {
    /// The folder of `session_id` under `changes_dir` (created when missing).
    ///
    /// # Errors
    /// `Refused` for an id that cannot be a folder name; `Io` when the folder cannot be created.
    pub fn open(changes_dir: &Path, session_id: &str) -> Result<Self, ChangesError> {
        if !valid_session_id(session_id) {
            return Err(ChangesError::Refused(format!(
                "session id {session_id:?} cannot name a data folder"
            )));
        }
        let dir = changes_dir.join("sessions").join(session_id);
        std::fs::create_dir_all(dir.join("blobs"))
            .map_err(|e| ChangesError::io("create", &dir, e))?;
        Ok(Self { dir })
    }

    /// A sub-folder used as its own session-like folder (the "against the last commit" view of a session
    /// whose baseline was lost). Created when missing.
    ///
    /// # Errors
    /// `Io` when the folder cannot be created.
    pub fn child(&self, name: &str) -> Result<Self, ChangesError> {
        let dir = self.dir.join(name);
        std::fs::create_dir_all(dir.join("blobs"))
            .map_err(|e| ChangesError::io("create", &dir, e))?;
        Ok(Self { dir })
    }

    /// True when a baseline was ever stored here (`state.json` exists, readable or not).
    #[must_use]
    pub fn has_state(&self) -> bool {
        self.dir.join("state.json").exists()
    }

    /// The folder itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// The baseline index.
    #[must_use]
    pub fn base_index(&self) -> PathBuf {
        self.dir.join("base.idx")
    }

    /// The working-tree index.
    #[must_use]
    pub fn current_index(&self) -> PathBuf {
        self.dir.join("cur.idx")
    }

    /// A scratch index for one operation (removed by the caller).
    #[must_use]
    pub fn scratch_index(&self) -> PathBuf {
        self.dir.join("scratch.idx")
    }

    /// Saved blob copies.
    #[must_use]
    pub fn blobs(&self) -> PathBuf {
        self.dir.join("blobs")
    }

    /// Reads `state.json` (`None` when there is none or it is unreadable or of another version).
    #[must_use]
    pub fn read_state(&self) -> Option<BaselineState> {
        let bytes = std::fs::read(self.dir.join("state.json")).ok()?;
        let state: BaselineState = serde_json::from_slice(&bytes).ok()?;
        (state.version == STATE_VERSION).then_some(state)
    }

    /// Replaces `state.json` atomically.
    ///
    /// # Errors
    /// `Io` on write or rename failures.
    pub fn write_state(&self, state: &BaselineState) -> Result<(), ChangesError> {
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|e| ChangesError::Parse(format!("state.json: {e}")))?;
        let tmp = self.dir.join("state.json.tmp");
        let path = self.dir.join("state.json");
        std::fs::write(&tmp, bytes).map_err(|e| ChangesError::io("write", &tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| ChangesError::io("replace", &path, e))
    }

    /// Removes the session's review data (the baseline is gone afterwards).
    ///
    /// # Errors
    /// `Io` when the folder exists but cannot be removed.
    pub fn remove(self) -> Result<(), ChangesError> {
        remove_folder(&self.dir)
    }

    /// Removes the folder of `session_id` under `changes_dir` if it exists, never creating it first (a
    /// removal through [`SessionDir::open`] created the folder for a moment, visible to anyone looking).
    ///
    /// # Errors
    /// `Refused` for an id that cannot be a folder name; `Io` when it exists but cannot be removed.
    pub fn remove_existing(changes_dir: &Path, session_id: &str) -> Result<(), ChangesError> {
        if !valid_session_id(session_id) {
            return Err(ChangesError::Refused(format!(
                "session id {session_id:?} cannot name a data folder"
            )));
        }
        remove_folder(&changes_dir.join("sessions").join(session_id))
    }

    /// Renames the folder of session `old` to `new`'s under `changes_dir` (a session id hand-off: the conversation
    /// continues as `new`, so its review baseline does too). Returns `false` when `old` has no folder or `new`
    /// already has one (that one is kept; nothing is overwritten).
    ///
    /// # Errors
    /// `Refused` for an id that cannot be a folder name; `Io` when the rename fails.
    pub fn rename_existing(changes_dir: &Path, old: &str, new: &str) -> Result<bool, ChangesError> {
        for id in [old, new] {
            if !valid_session_id(id) {
                return Err(ChangesError::Refused(format!(
                    "session id {id:?} cannot name a data folder"
                )));
            }
        }
        let sessions = changes_dir.join("sessions");
        let (from, to) = (sessions.join(old), sessions.join(new));
        if !from.is_dir() || to.exists() {
            return Ok(false);
        }
        std::fs::rename(&from, &to).map_err(|e| ChangesError::io("rename", &from, e))?;
        Ok(true)
    }
}

fn remove_folder(dir: &Path) -> Result<(), ChangesError> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ChangesError::io("remove", dir, e)),
    }
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
