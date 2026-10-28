//! Bounded folder browsing and commit history requests. Used by the Files panel through the Git browser command.
use serde::{Deserialize, Serialize};
use vw_git::browser_status::BrowserStatus;

/// A read or confirmed folder action, scoped to a known folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ExplorerRequest {
    /// Read just one directory, with a bounded continuation.
    Folder {
        /// Root-relative path.
        path: String,
        /// Include ignored entries when requested.
        show_ignored: bool,
        /// Continuation offset.
        offset: u32,
    },
    /// Read repository state separately from folder first paint.
    Status {},
    /// Build the bounded file-name index separately from first paint.
    Index {},
    /// Search file contents; a newer request cancels the prior search.
    Search {
        /// Literal search words.
        query: String,
        /// Only inside this root-relative folder (Search in this folder), so the hit cap counts there.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        path: Option<String>,
    },
    /// Cancel a pending content search.
    Cancel {},
    /// Read fifty commits anchored to the first page's head.
    History {
        /// Opaque continuation from the previous page.
        cursor: Option<String>,
        /// Literal search words.
        query: String,
        /// Only the commits that changed this root-relative file (History of this file).
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        path: Option<String>,
    },
    /// One file's text as it was in a commit (bounded, editable text only), for a compare beside the file now.
    ReadAt {
        /// Commit object id.
        id: String,
        /// Root-relative path.
        path: String,
    },
    /// Read a commit's changed paths, without reading file contents.
    Version {
        /// Commit object id.
        id: String,
    },
    /// Restore the named working files after confirmation in the window.
    Revert {
        /// HEAD observed before review.
        head: String,
        /// Fingerprint the review showed.
        fingerprint: String,
        /// New files the confirmation said would be deleted.
        delete_paths: Vec<String>,
    },
    /// Read changes since HEAD without a session baseline.
    Review {},
    /// Preview turning on version history.
    PlanHistory {},
    /// Create the first commit after the user confirms the plan.
    EnableHistory {
        /// The user accepted the initialization plan.
        confirmed: bool,
        /// Author details for the first commit.
        identity: vw_git::identity::CommitIdentity,
    },
}

/// One immediate child; directories are never recursively read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExplorerEntry {
    /// Root-relative path.
    pub path: String,
    /// Directory or regular file.
    pub directory: bool,
}

/// A commit, using the author's words and a plain publication location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExplorerVersion {
    /// Object id used only for subsequent reads.
    pub id: String,
    /// Subject.
    pub subject: String,
    /// Author name, or you when local identity matches.
    pub who: String,
    /// Unix seconds.
    pub time: f64,
    /// Whether the local upstream contains this commit; None if no upstream is known.
    pub published: Option<bool>,
}

/// One file-content search match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExplorerHit {
    /// Relative path.
    pub path: String,
    /// One-based source line.
    pub line: u32,
    /// Bounded source text.
    pub text: String,
}

/// Bounded results for one Files operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExplorerResult {
    /// Immediate directory entries and current git status.
    Folder {
        /// Immediate children.
        entries: Vec<ExplorerEntry>,
        /// Ignored names omitted by default.
        ignored: Vec<String>,
        /// Offset for the next bounded directory page.
        next: Option<u32>,
        /// Current repository status, absent for a plain folder.
        status: Option<BrowserStatus>,
    },
    /// Repository state and a notice when history is unavailable.
    Status {
        /// Current status, absent outside a repository.
        status: Option<BrowserStatus>,
        /// Explanation when repository metadata exists but Git cannot read it.
        note: Option<String>,
        /// Number of commits reachable from this branch.
        commits: Option<u32>,
        /// Newest commit, when present.
        latest: Option<ExplorerVersion>,
    },
    /// Background name list, never file contents.
    Index {
        /// Root-relative file names.
        paths: Vec<String>,
        /// More results exist beyond the reading limit.
        truncated: bool,
    },
    /// Content matches.
    Search {
        /// Source-line matches.
        hits: Vec<ExplorerHit>,
        /// More results exist beyond the reading limit.
        truncated: bool,
    },
    /// A stable page of fifty commits.
    History {
        /// Newest-first commits.
        versions: Vec<ExplorerVersion>,
        /// Opaque continuation from the previous page.
        cursor: Option<String>,
    },
    /// Changed paths of one commit.
    Version {
        /// Paths changed in the commit.
        files: Vec<String>,
        /// More results exist beyond the reading limit.
        truncated: bool,
    },
    /// A file's text in a commit, or why it cannot be shown.
    Text {
        /// The text (LF line endings), absent when it cannot be compared.
        text: Option<String>,
        /// Why not, in the user's words.
        note: Option<String>,
    },
    /// Current changes since the last commit.
    Review {
        /// Changes since HEAD.
        view: crate::changes_view::ChangesView,
        /// HEAD observed before review.
        head: String,
    },
    /// Existing version history initialization plan.
    InitPlan {
        /// Files and consequence of enabling version history.
        plan: vw_changes::InitPlan,
    },
    /// Action completed or search cancelled.
    Done {},
}
