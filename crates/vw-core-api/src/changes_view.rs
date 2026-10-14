//! The `changes` command's requests and results: what the Changes tab and the Debug API send and
//! get back. Logic lives in `changes.rs`; the git work in the vw-changes crate.
//!
//! Depends on: vw-changes (diff types, targets, init plan), serde. Called by: `command` (the `changes`
//! variant), `changes`, the UI through the generated TypeScript.
//! Invariants:
//! - Wire form like every command: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - Views never carry file contents beyond the diff lines of text files; hunks' raw bytes stay in the core.

use serde::{Deserialize, Serialize};
use vw_changes::{FileChange, InitPlan, Target};

use crate::error::{CoreError, ErrorCode};

/// One Changes request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChangesRequest {
    /// The session's changes now (refreshes first).
    Read {
        /// Session id.
        session_id: String,
    },
    /// Accept a hunk, a file or everything: the review baseline moves; the working tree is not touched.
    Keep {
        /// Session id.
        session_id: String,
        /// What to keep, with the blob/hunk id or fingerprint the user saw.
        target: Target,
    },
    /// Put a hunk, a file or everything back the way it was when the review started (working tree only).
    Revert {
        /// Session id.
        session_id: String,
        /// What to revert, with the blob/hunk id or fingerprint the user saw.
        target: Target,
        /// Files the revert may delete (created during the session); must name every one it would delete.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        delete_paths: Option<Vec<String>>,
    },
    /// Folders without git: what undoing a turn would restore (Vibe's rewind data) and what it cannot.
    PreviewUndo {
        /// Session id.
        session_id: String,
        /// The turn to undo (every later turn is undone with it).
        turn_id: String,
    },
    /// Loaded sessions that work in `cwd`'s repository (or folder) and are running or have changes: a new
    /// session there is offered its own worktree.
    FolderSessions {
        /// The folder a new session is about to start in.
        cwd: String,
    },
    /// What turning on version history in the session's folder would commit (for the confirmation).
    PlanHistory {
        /// Session id.
        session_id: String,
    },
    /// Runs `git init` and a first commit in the session's folder. `confirmed` must be true: the user saw
    /// the confirmation built from `planHistory`.
    EnableHistory {
        /// Session id.
        session_id: String,
        /// The user confirmed in the window.
        confirmed: bool,
    },
    /// "Don't ask again" for the version-history offer in the session's folder.
    DismissOffer {
        /// Session id.
        session_id: String,
    },
    /// Takes a new baseline now ("Start reviewing from now", after the stored one was lost). Explicit only:
    /// the core never re-baselines on its own.
    Restart {
        /// Session id.
        session_id: String,
    },
}

impl ChangesRequest {
    /// The op name as on the wire, for logs.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Read { .. } => "read",
            Self::Keep { .. } => "keep",
            Self::Revert { .. } => "revert",
            Self::PreviewUndo { .. } => "previewUndo",
            Self::FolderSessions { .. } => "folderSessions",
            Self::PlanHistory { .. } => "planHistory",
            Self::EnableHistory { .. } => "enableHistory",
            Self::DismissOffer { .. } => "dismissOffer",
            Self::Restart { .. } => "restart",
        }
    }
}

/// What a Changes request returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ChangesResult {
    /// `read`, `keep`, `revert`, `enableHistory`, `dismissOffer`.
    View {
        /// The session's changes after the request.
        view: ChangesView,
    },
    /// `previewUndo`.
    UndoPreview {
        /// What the undo would do.
        preview: UndoPreview,
    },
    /// `folderSessions`.
    Folder {
        /// Whether the folder is in a git repository (a worktree is only possible then).
        is_git: bool,
        /// Live sessions working there.
        sessions: Vec<FolderSession>,
    },
    /// `planHistory`.
    InitPlan {
        /// What the first commit would contain.
        plan: InitPlan,
    },
}

/// Which kind of Changes view a session has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ChangesMode {
    /// Nothing changed yet (and no git review started): no tab, no git mention.
    None,
    /// A git folder: diff against the session's baseline, keep/revert per hunk.
    Git,
    /// A folder without git: changes from Vibe's file edits, undo per turn.
    Effects,
    /// A git folder whose review cannot run (`reason` says why).
    Unavailable,
}

/// One session's changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChangesView {
    /// Session id.
    pub session_id: String,
    /// Which view.
    pub mode: ChangesMode,
    /// The session's folder.
    pub folder: Option<String>,
    /// The repository root (git mode).
    pub repo_root: Option<String>,
    /// The reviewed subtree relative to the root (`""` = the whole repository).
    pub scope: Option<String>,
    /// Why the review cannot run (`unavailable`), or why the baseline was lost (`baseline_lost`).
    pub reason: Option<String>,
    /// The session's starting point was lost or tampered with: `files` are against the last commit, read
    /// only, until the user starts reviewing again (`restart`).
    pub baseline_lost: bool,
    /// When the review started (ms since the epoch).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub baseline_at_ms: Option<u64>,
    /// Changed files (git mode).
    pub files: Vec<FileChange>,
    /// The diff's fingerprint, for `keep`/`revert` of everything.
    pub fingerprint: String,
    /// Things to tell the user (files not shown and why).
    pub notes: Vec<String>,
    /// Turns that changed files (effects mode), oldest first.
    pub turns: Vec<TurnChanges>,
    /// Show the one-time "turn on version history" line (effects mode, code folder, not dismissed).
    pub offer_history: bool,
    /// A file watcher refreshes this view live.
    pub watching: bool,
    /// How long the refresh took (ms).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub took_ms: u64,
}

/// One turn's file changes in a folder without git.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TurnChanges {
    /// Vibe's turn id.
    pub turn_id: String,
    /// 1-based turn number.
    pub number: u32,
    /// The user entry that opened the turn (the rewind anchor), when known.
    pub anchor_entry_id: Option<String>,
    /// Files the turn wrote or edited through Vibe's file tools.
    pub files: Vec<TurnFile>,
}

/// One file a turn changed (folders without git).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TurnFile {
    /// Path relative to the session folder when inside it, else as Vibe gave it.
    pub path: String,
    /// The effect entries (render them with the transcript's diff view).
    pub entry_ids: Vec<String>,
    /// The turn created the file (a `file_write` of a file that did not exist).
    pub created: bool,
}

/// What undoing a turn would do (folders without git).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UndoPreview {
    /// The turn.
    pub turn_id: String,
    /// The rewind anchor (`rewindSession` takes it as `entryId`).
    pub anchor_entry_id: String,
    /// Files Vibe will put back (relative to the session folder where possible).
    pub restorable: Vec<String>,
    /// Files changed in this turn or later that Vibe cannot put back (no copy from before).
    pub not_restorable: Vec<String>,
    /// Turns removed from the conversation (this one and every later one).
    pub turns_removed: u32,
}

/// A live session in a folder (two sessions, one folder).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FolderSession {
    /// Session id.
    pub session_id: String,
    /// Its title, when it has one.
    pub title: Option<String>,
    /// A turn is running.
    pub running: bool,
    /// Files it has changed (0 when unknown).
    pub changed_files: u32,
}

/// Maps a vw-changes error to a core error. `stale` and `confirmDeletes` are user states; `confirmDeletes`
/// lists the files in `summary`, one per line.
pub(crate) fn core_error(error: vw_changes::ChangesError) -> CoreError {
    use vw_changes::ChangesError as E;
    match error {
        E::Stale { path } => {
            CoreError::new(ErrorCode::Stale, format!("File changed, refresh ({path})"))
        }
        E::ConfirmDeletes { paths } => {
            let mut error = CoreError::new(
                ErrorCode::ConfirmDeletes,
                format!(
                    "Reverting deletes {}: confirm by naming them in deletePaths",
                    paths.join(", ")
                ),
            );
            error.summary = Some(paths.join("\n"));
            error
        }
        E::NotFound(what) => CoreError::new(ErrorCode::NotFound, what),
        E::Refused(why) => CoreError::new(ErrorCode::Denied, why),
        other => CoreError::new(ErrorCode::Internal, other.to_string()),
    }
}
