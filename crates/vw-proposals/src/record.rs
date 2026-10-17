//! Bounded session entries from immutable reviews and final application outcomes.
//! Uses serde; called by apply. Retains no full file contents or filesystem permissions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{ContentKind, Error, Kind, MAX_RECORD_DIFF_BYTES, Proposal, Status};

/// A final proposal decision, ready to append to the owning session's record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Record {
    /// Proposal identity, unchanged across review and decision.
    pub proposal_id: String,
    /// Owning session.
    pub session_id: String,
    /// Proposal creation time, milliseconds since the Unix epoch.
    pub created_at_ms: u64,
    /// Agent's bounded plain-text explanation.
    pub reason: String,
    /// Files and bounded diff text in proposal order.
    pub files: Vec<RecordFile>,
    /// Approved, declined, stale or failed.
    pub decision: Status,
    /// Decision time, milliseconds since the Unix epoch.
    pub decided_at_ms: u64,
    /// Successfully applied paths in order; empty on decline and stale.
    pub applied: Vec<PathBuf>,
    /// Explanation for a stale or failed approval.
    pub problem: Option<RecordProblem>,
}

/// File metadata and its share of the record's combined diff-text cap.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordFile {
    /// Add, edit or remove.
    pub kind: Kind,
    /// Relative path.
    pub path: PathBuf,
    /// Text or binary.
    pub content_kind: ContentKind,
    /// Original byte count.
    pub before_bytes: usize,
    /// Proposed byte count.
    pub after_bytes: usize,
    /// Full review's added text lines; zero for binary.
    pub added_lines: usize,
    /// Full review's removed text lines; zero for binary.
    pub removed_lines: usize,
    /// Full unified diff byte count, even when the stored text is truncated.
    pub diff_bytes: usize,
    /// UTF-8 prefix of the unified diff, or None for binary.
    pub diff: Option<String>,
    /// The record omitted some of this file's diff text.
    pub diff_truncated: bool,
}

/// Failure details with no file bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordProblem {
    /// Refused or failed path when available.
    pub path: Option<PathBuf>,
    /// Human-readable error; never includes file contents.
    pub message: String,
}

impl Record {
    pub(crate) fn new(proposal: &Proposal, decision: Status) -> Self {
        let mut remaining = MAX_RECORD_DIFF_BYTES;
        let files = proposal
            .review()
            .into_iter()
            .map(|file| {
                let diff_bytes = file.diff.as_ref().map_or(0, String::len);
                let diff = file.diff.map(|mut text| {
                    let mut end = remaining.min(text.len());
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    text.truncate(end);
                    remaining -= end;
                    text
                });
                let kept = diff.as_ref().map_or(0, String::len);
                RecordFile {
                    kind: file.kind,
                    path: file.path,
                    content_kind: file.content_kind,
                    before_bytes: file.before_bytes,
                    after_bytes: file.after_bytes,
                    added_lines: file.added_lines,
                    removed_lines: file.removed_lines,
                    diff_bytes,
                    diff,
                    diff_truncated: kept < diff_bytes,
                }
            })
            .collect();
        Self {
            proposal_id: proposal.id.clone(),
            session_id: proposal.session_id.clone(),
            created_at_ms: proposal.created_at_ms,
            reason: proposal.reason.clone(),
            files,
            decision,
            decided_at_ms: crate::now_ms(),
            applied: Vec::new(),
            problem: None,
        }
    }

    pub(crate) fn fail(&mut self, error: &Error) {
        self.decision = if matches!(error, Error::Stale { .. }) {
            Status::Stale
        } else {
            Status::Failed
        };
        self.problem = Some(RecordProblem {
            path: error.path().map(std::path::Path::to_owned),
            message: error.to_string(),
        });
    }
}
