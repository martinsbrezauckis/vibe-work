//! Worktree per session: a session can work in its own copy of a git
//! repository (a git worktree Vibe makes and manages), on its own branch started from a base the user picked,
//! and that copy is removed only after the user saw what would be lost.
//!
//! This crate is the git side, without Vibe's protocol: the folder's repository and its branches
//! ([`repo_info`]), the preflight that refuses repositories whose own settings would make git run a program
//! while a copy is made, the branch name suggested from the first message ([`naming`]), the session branch
//! created at the base before Vibe is asked for the copy ([`branch`]), what a removal would lose
//! ([`inspect`]), and the per-session records ([`records`]).
//!
//! Depends on: vw-changes (the one hardened git runner and its filter-free working-tree reading),
//! vw-platform (user-only folders), serde. Called by: vw-core-api (`worktrees` command).
//! Invariants:
//! - Every git call goes through `vw_changes::git::Call`: hardened config, allowlisted environment, no
//!   repository-defined program, no network.
//! - Nothing here writes the user's index, `HEAD`, stash or config; the only repository writes are the
//!   session branch (create-only, deleted only while untouched) and unreachable loose objects of the
//!   inspection.
//! - Nothing here removes a worktree: removal is Vibe's (`workspace/git/worktrees/remove` with explicit
//!   parameters), asked for by the core after [`inspect`] and the user's confirmation.

pub mod branch;
mod error;
pub mod include;
pub mod inspect;
pub mod naming;
pub mod records;
pub mod repo_info;

pub use error::WorktreeError;
pub use include::{IncludeFile, IncludeReport, IncludeSkip, SkipReason};
pub use inspect::{Inspection, LostFile, LostKind, inspect};
pub use naming::{Suggestion, suggest};
pub use records::{Decision, Record, Records};
pub use repo_info::{BranchInfo, Head, RepoInfo, WorktreeEntry, preflight, read};

/// One-line statement of what this crate is responsible for.
pub const CRATE_ROLE: &str =
    "worktree per session: base branches, session branch, preflight, inspection, records";
