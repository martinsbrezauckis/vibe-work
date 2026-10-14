//! The `lineComments` command's requests and results: comments on lines of a session's diff.
//! Logic lives in `line_comments.rs`; the store and the anchor in the vw-changes crate.
//!
//! Depends on: vw-changes (comment, anchor types), serde. Called by: `command` (the `lineComments` variant),
//! `line_comments`, the UI through the generated TypeScript (`changes.ts`).
//! Invariants: wire form like every command (`{"op": "<camelCase>", ...}`, unknown fields refused); a request
//! never carries the quoted lines: the core takes them from its own diff.

use serde::{Deserialize, Serialize};
use vw_changes::LineComment;

/// One line-comment request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LineCommentsRequest {
    /// The session's comments, each placed in the diff as it is now or marked outdated.
    List {
        /// Session id.
        session_id: String,
    },
    /// Comments on lines `from..=to` (indexes into the hunk's lines) of hunk `hunk_id` of `path`.
    Add {
        /// Session id.
        session_id: String,
        /// The file as the review shows it.
        path: String,
        /// The hunk the user saw (refused as stale when the diff moved on).
        hunk_id: String,
        /// First commented line (index in the hunk).
        from: u32,
        /// Last commented line (index in the hunk, inclusive).
        to: u32,
        /// The comment.
        text: String,
    },
    /// Deletes a comment.
    Remove {
        /// Session id.
        session_id: String,
        /// Comment id.
        id: String,
    },
    /// Marks comments as put into the composer (the UI inserted their chips).
    MarkSent {
        /// Session id.
        session_id: String,
        /// Comment ids.
        ids: Vec<String>,
    },
}

impl LineCommentsRequest {
    /// The session the request is about.
    #[must_use]
    pub fn session_id(&self) -> &str {
        match self {
            Self::List { session_id }
            | Self::Add { session_id, .. }
            | Self::Remove { session_id, .. }
            | Self::MarkSent { session_id, .. } => session_id,
        }
    }
}

/// Where a comment is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LineCommentState {
    /// Its lines are in the diff: hunk and line indexes in the view just read.
    Attached {
        /// The hunk's id in the current diff.
        hunk_id: String,
        /// Index of the first commented line in the hunk.
        from: u32,
        /// Index of the last commented line.
        to: u32,
    },
    /// Its lines are not in the diff any more (edited, kept, reverted); the comment is kept.
    Outdated {
        /// Why, in plain words.
        reason: String,
    },
}

/// A comment and where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LineCommentView {
    /// The comment.
    pub comment: LineComment,
    /// Where it is now.
    #[serde(flatten)]
    pub place: LineCommentState,
}

/// What a line-comment request returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LineCommentsResult {
    /// `list`.
    List {
        /// The session's comments, oldest first.
        comments: Vec<LineCommentView>,
    },
    /// `add`.
    Added {
        /// The new comment.
        comment: Box<LineCommentView>,
    },
    /// `remove`, `markSent`.
    Done {},
}
