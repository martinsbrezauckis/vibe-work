//! The `worktrees` command's requests and results: what the new-session panel, the session
//! header, the end-of-session dialog, the startup list and the Debug API send and get back. Logic lives in
//! `worktrees.rs`, `worktrees_start.rs` and `worktrees_remove.rs`; the git work in the vw-worktrees crate.
//!
//! Depends on: vw-worktrees (`Head`, `BranchInfo`, `LostFile`, `Decision`), `view` (`SessionSnapshot`),
//! serde. Called by: `command` (the `worktrees` variant), the worktree modules, the UI through the
//! generated TypeScript.
//! Invariant: wire form like every command: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields
//! refused.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vw_worktrees::{BranchInfo, Head, IncludeReport, LostFile};

use crate::view::SessionSnapshot;

/// One worktree request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorktreesRequest {
    /// Number of extant linked session copies Vibe Work made, including kept and orphaned copies.
    Count {},
    /// What a new session in `cwd` may do: is it git, its branches (base choices), is a copy offered.
    Options {
        /// The folder the new session is about to start in.
        cwd: String,
    },
    /// The branch suggested for a first message (free: neither the branch nor the folder exists).
    SuggestName {
        /// The folder the new session is about to start in.
        cwd: String,
        /// The first message (may be empty).
        text: String,
    },
    /// Starts a session in its own copy: creates `branch` at `base`, then `session/start` with
    /// `agentConfig.worktree = {kind: "create", branch, name}`. The Debug API's confirmation is answered by the core (no dialog).
    Start {
        /// The session's `AgentConfig` (its `cwd` names the folder; it may not carry `worktree`).
        agent_config: Value,
        /// The base: a local branch name, or the commit id of a detached checkout.
        base: String,
        /// The new branch (must not exist).
        branch: String,
    },
    /// The session's copy for the header (none when the session works in its folder).
    Info {
        /// Session id.
        session_id: String,
    },
    /// What removing a session's copy would lose. Only copies Vibe Work made (its records). From the Debug API
    /// the answer holds counts, not names or paths; the window gets the lists.
    Inspect {
        /// Session id.
        session_id: String,
    },
    /// Removes a session's copy. `fingerprint` must equal the inspection the user saw (else `stale`); the
    /// native dialog names the files (every origin). The branch goes only when Vibe Work created it and no
    /// commit on it is missing from every other branch and remote.
    Remove {
        /// Session id.
        session_id: String,
        /// `fingerprint` of the inspection the user saw.
        fingerprint: String,
    },
    /// Copies Vibe Work made whose session was deleted and that nobody decided about (startup list).
    Orphans {},
}

impl WorktreesRequest {
    /// The op name as on the wire, for logs.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Count {} => "count",
            Self::Options { .. } => "options",
            Self::SuggestName { .. } => "suggestName",
            Self::Start { .. } => "start",
            Self::Info { .. } => "info",
            Self::Inspect { .. } => "inspect",
            Self::Remove { .. } => "remove",
            Self::Orphans {} => "orphans",
        }
    }
}

/// What a worktree request returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WorktreesResult {
    /// `count`.
    Count {
        /// Number of extant linked session copies.
        count: usize,
    },
    /// `options`.
    Options {
        /// The answer.
        options: WorktreeOptions,
    },
    /// `suggestName`.
    Name {
        /// `vibe/<slug>`.
        branch: String,
        /// The worktree folder name that goes with it.
        folder: String,
    },
    /// `start`.
    Started {
        /// The new session (as `startSession` returns it).
        snapshot: SessionSnapshot,
        /// Its copy.
        info: WorktreeInfo,
        /// What `.worktreeinclude` copied into it and what it skipped (names only).
        include: IncludeReport,
        /// One setup line from package files in the new copy's root, for its first task.
        setup: Option<String>,
    },
    /// `info`.
    Info {
        /// The copy, `null` when the session works in its folder.
        info: Option<WorktreeInfo>,
    },
    /// `inspect`.
    Inspection {
        /// What a removal would lose and whether it can happen.
        inspection: WorktreeInspection,
    },
    /// `remove`.
    Removed {
        /// What happened.
        outcome: RemoveOutcome,
    },
    /// `orphans`.
    Orphans {
        /// Undecided leftovers and kept copies, oldest first.
        orphans: Vec<OrphanWorktree>,
    },
}

/// What a new session in a folder may do. Only a folder the user picked in the native dialog in this run is
/// read; for any other folder `picked` is false and nothing else is filled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorktreeOptions {
    /// The folder was picked natively in this run. Otherwise only an exact recorded copy is recognized.
    pub picked: bool,
    /// Branch of an existing Vibe Work managed copy at this folder, when recognized.
    pub existing_branch: Option<String>,
    /// The folder is inside a git working tree (false also when it was not read).
    pub is_git: bool,
    /// Root of the repository's main working tree (window only; `null` for the Debug API).
    pub repo_root: Option<String>,
    /// The folder's `HEAD`.
    pub head: Option<Head>,
    /// Local branches (base choices), the current one first. For the Debug API the names only (`commit`
    /// empty).
    pub branches: Vec<BranchInfo>,
    /// A copy may be made.
    pub offered: bool,
    /// Why not (plain words), when it is git but not offered.
    pub reason: Option<String>,
    /// The suggestion for an empty first message.
    pub suggestion: Option<String>,
    /// What a copy started now would take from `.worktreeinclude` (names only, `null` when no copy
    /// is offered or the folder was not read).
    pub include: Option<IncludeReport>,
}

/// Where a session's copy stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum WorktreeState {
    /// Asked for; Vibe has not made the folder yet.
    Pending,
    /// The folder exists.
    Ready,
    /// The folder is gone (removed outside Vibe Work, or making it failed).
    Missing,
    /// Removed through Vibe Work.
    Removed,
}

/// A session's copy, for the header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    /// The session.
    pub session_id: String,
    /// Its branch.
    pub branch: String,
    /// The base it started from (`null` for a copy Vibe Work did not make, e.g. a fork's).
    pub base: Option<String>,
    /// The copy's folder (`null` while pending, and for the Debug API).
    pub path: Option<String>,
    /// Where it stands.
    pub state: WorktreeState,
    /// Vibe Work made it (a record exists).
    pub made_here: bool,
}

/// What happens to the branch when the copy is removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BranchPlan {
    /// Deleted with the folder (Vibe Work made it and it has no new commits).
    Deleted,
    /// Kept (why, in plain words).
    Kept {
        /// For the dialog.
        reason: String,
    },
}

/// A copy's state before a removal, with its lists (names, paths, commits) for every origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInspection {
    /// The session.
    pub session_id: String,
    /// The copy's folder.
    pub path: Option<String>,
    /// The folder exists.
    pub exists: bool,
    /// Its branch.
    pub branch: Option<String>,
    /// The base it started from.
    pub base: Option<String>,
    /// Paths whose unsaved state a removal discards (window only).
    pub files: Vec<LostFile>,
    /// How many there are.
    pub file_count: u32,
    /// Commits no other branch or remote contains (window only; `<id> <subject>`).
    pub unmerged: Vec<String>,
    /// How many there are.
    pub unmerged_total: u32,
    /// Ignored build output that goes with the folder (counted only).
    pub build_output: u32,
    /// Stash entries made on the branch (window only; they stay in the stash list).
    pub stashes: Vec<String>,
    /// How many there are.
    pub stash_count: u32,
    /// What happens to the branch.
    pub branch_plan: BranchPlan,
    /// Why it cannot be removed now (plain words); `null` when it can.
    pub blocked: Option<String>,
    /// Other sessions holding it (Vibe's count), when Vibe was asked.
    pub holders: Option<u32>,
    /// Names this exact state for `remove`.
    pub fingerprint: String,
}

/// What a removal did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RemoveOutcome {
    /// The folder is gone.
    pub removed: bool,
    /// The branch is gone.
    pub branch_deleted: bool,
    /// Vibe's outcome word (`removed`, `kept_in_use`, ...).
    pub vibe_outcome: String,
    /// Vibe's reasons when it kept the copy.
    pub reasons: Vec<String>,
}

/// A leftover or kept copy available for later review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct OrphanWorktree {
    /// The original session's id.
    pub session_id: String,
    /// The copy's folder (window only).
    pub path: Option<String>,
    /// Its branch.
    pub branch: String,
    /// Its base.
    pub base: String,
    /// The repository (window only; `null` for the Debug API).
    pub repo_root: Option<String>,
    /// When the copy was made (ms since the epoch).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_at_ms: u64,
}
