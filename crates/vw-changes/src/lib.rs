//! The Changes tab's core: what an agent session changed in its folder, and the
//! user's keep or revert per hunk, per file or for everything.
//!
//! Git folders: a per-session **baseline** tree built through a temporary index in our app data
//! ([`Baseline`]), the working tree diffed against it ([`refresh`]), [`keep`] (the baseline moves) and
//! [`revert`] (the working tree moves back), a debounced file [`watch`]er. Git never reads or writes the
//! working tree here: we read files, store them with `hash-object --no-filters`, diff lines in Rust and
//! write reverted files ourselves, so a repository's filters, diff drivers and hooks never run. Folders without git: whether the
//! folder looks like code ([`looks_like_code`]) and the user-confirmed `git init` + first commit
//! ([`init_repository`]).
//!
//! Depends on: the `git` CLI (2.26+), vw-platform (user-only data folder), notify, serde. Knows nothing about
//! Vibe's protocol. Called by: vw-core-api (`changes` command, turn hooks, live refresh).
//! Invariants:
//! - Never writes the user's index, HEAD, branches, refs, stash or config; the only repository writes are
//!   loose objects (and, for `init_repository`, what the user confirmed).
//! - Never runs a program the repository names (filters, textconv, external diff, fsmonitor, hooks, pager):
//!   tested with a hostile repository (`vw_changes_hostile.rs`).
//! - Every action checks that its target is what the user saw; otherwise nothing is written (`Stale`).

mod baseline;
mod diff;
mod error;
mod files;
pub mod git;
mod hooks;
mod ignoresnap;
mod line_anchor;
mod line_comments;
mod linediff;
mod model;
mod objects;
mod parse;
mod paths;
mod plain;
mod repo;
mod review;
mod statcache;
mod store;
mod watch;
mod worktree;

pub use baseline::{Baseline, FULL_PASS, Loaded, against_head, diff_against_head};
pub use diff::{MAX_CHANGED_LINES, MAX_COMPARE_BYTES, MAX_DIFF_BYTES, refresh};
pub use error::ChangesError;
pub use git::version as git_version;
pub use line_anchor::{AnchorError, LineAnchor, Located, MAX_LINES, anchor_for, line_hash, locate};
pub use line_comments::{
    LineComment, LineCommentError, LineCommentStore, MAX_PER_SESSION, MAX_TEXT, session_key,
};
pub use model::{
    ContentKind, Diff, DiffLine, FileChange, FileStatus, Hunk, LineKind, Target, TurnLabel,
    TurnSource,
};
pub use parse::{RawEntry, fnv_hex};
pub use paths::{display, has_symlinked_parent, inside, inside_bytes, quote, unquote};
pub use plain::{
    FALLBACK_EMAIL, FALLBACK_NAME, InitPlan, MANIFESTS, SOURCE_EXTENSIONS, dismiss_offer,
    init_repository, looks_like_code, offer_dismissed, plan_init,
};
pub use repo::{MIN_GIT, Repo, discover, head_tree};
pub use review::{keep, revert};
pub use store::{BaselineState, STATE_VERSION, SessionDir, now_ms, prepare_root, valid_session_id};
pub use watch::{Debounce, Watcher, watch};
pub use worktree::{LARGE_UNTRACKED, LargeFile, MAX_UNTRACKED, ignored, watch_ignored};

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics can assert
/// which crate they are looking at.
pub const CRATE_ROLE: &str = "changes review: git baseline, diff by hunk, keep and revert";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
