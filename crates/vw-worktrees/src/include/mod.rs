//! `.worktreeinclude`: copy gitignored files such as `.env` from the main checkout into a new
//! session worktree. The file follows the convention other agent tools use: `.gitignore` syntax, and only
//! git-ignored files that a pattern selects are copied.
//!
//! The file sits at the root of the main checkout and uses `.gitignore` syntax ([`pattern`]). A path is copied
//! only when a pattern selects it AND git says it is ignored in the main checkout, so tracked files are never
//! duplicated ([`plan`]); the copy itself ([`copy`]) writes only regular files below the new worktree's folder.
//! The repository is treated as hostile input: nothing here runs repository code, follows a link out of the
//! checkout, or writes anywhere but into the folder it was given.
//!
//! Depends on: vw-changes (the hardened git runner), serde. Called by: vw-core-api (`worktrees start` copies,
//! `worktrees options` previews).
//! Invariants:
//! - Names only ever leave this module (paths, sizes, reasons), never file contents.
//! - Over a cap ([`MAX_FILES`], [`MAX_BYTES`], [`MAX_VISITED`]) nothing is copied and the report says so.
//! - No function here panics or fails the session start: problems become `stopped` / `skipped` in the report.

use serde::{Deserialize, Serialize};

mod copy;
mod names;
pub mod pattern;
mod plan;
mod safefs;
mod source;

pub use copy::apply;
pub use names::validate_relative;
pub use plan::plan;

/// The file at the root of the main checkout.
pub const INCLUDE_FILE: &str = ".worktreeinclude";
/// Most files one copy may hold. `.env`-class files are a few KB; this leaves room for a local config tree and
/// is far below any dependency folder (a `node_modules` is an install, not a copy target).
pub const MAX_FILES: usize = 5_000;
/// Most bytes one copy may hold (200 MiB, same reasoning as [`MAX_FILES`]).
pub const MAX_BYTES: u64 = 200 << 20;
/// Most directory entries one plan may look at (a hostile or huge ignored folder cannot stall the start).
pub const MAX_VISITED: usize = 100_000;
/// Deepest directory nesting the plan descends into.
pub(crate) const MAX_DEPTH: usize = 64;
/// Largest `.worktreeinclude` read.
pub(crate) const MAX_INCLUDE_FILE: u64 = 64 << 10;
/// Most patterns one `.worktreeinclude` may hold.
pub(crate) const MAX_PATTERNS: usize = 500;
/// Skipped entries listed in a report (the rest are counted).
pub(crate) const MAX_SKIPS_LISTED: usize = 200;

/// One file that is (plan) or was (result) copied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct IncludeFile {
    /// Path relative to the repository root, `/` separated.
    pub path: String,
    /// Size in bytes (at most [`MAX_BYTES`]).
    pub size: u32,
}

/// Why a selected file was not copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// A symlink, junction or other reparse point (never followed, never copied).
    Link,
    /// Not a regular file (a socket, device, folder).
    NotRegular,
    /// A name that could escape the folder or means something else on another system.
    UnsafeName,
    /// Not valid text as a file name.
    NotText,
    /// The worktree already has a file there (a checked-out file is never overwritten).
    Exists,
    /// It could not be read or written.
    Unreadable,
    /// It changed while the copy ran.
    Changed,
    /// It lies inside another working tree of the repository.
    InsideWorktree,
}

/// One selected file that was not copied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct IncludeSkip {
    /// The path (relative, `/` separated; lossy text for a name that is not valid text).
    pub path: String,
    /// Why.
    pub reason: SkipReason,
}

/// What a plan will copy, or what a copy did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct IncludeReport {
    /// A `.worktreeinclude` exists at the repository root.
    pub found: bool,
    /// Files to copy (plan) or copied (result), sorted by path.
    pub files: Vec<IncludeFile>,
    /// Their total size in bytes.
    pub bytes: u32,
    /// Selected files that are not copied, with the reason (at most [`MAX_SKIPS_LISTED`] listed).
    pub skipped: Vec<IncludeSkip>,
    /// How many files were skipped in all.
    pub skipped_total: u32,
    /// Why the whole copy did not happen or stopped (plain words); `null` when nothing went wrong.
    pub stopped: Option<String>,
}

impl IncludeReport {
    /// Records a skipped file.
    pub(crate) fn skip(&mut self, path: impl Into<String>, reason: SkipReason) {
        self.skipped_total = self.skipped_total.saturating_add(1);
        if self.skipped.len() < MAX_SKIPS_LISTED {
            self.skipped.push(IncludeSkip {
                path: path.into(),
                reason,
            });
        }
    }

    /// Records a file as copied (or to copy).
    pub(crate) fn add(&mut self, path: String, size: u64) {
        let size = u32::try_from(size).unwrap_or(u32::MAX);
        self.bytes = self.bytes.saturating_add(size);
        self.files.push(IncludeFile { path, size });
    }
}
