//! Typed protected-folder reviews shared by the window and Debug API.
//! Depends on serde and optional ts-rs; the proposal hub publishes these without file contents.
use serde::{Deserialize, Serialize};

/// A captured file's read-only diff and byte transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProtectedFileView {
    /// Relative to Vibe's home.
    pub path: String,
    /// Add, edit or remove.
    pub kind: String,
    /// Text or binary.
    pub content_kind: String,
    /// Original bytes.
    pub before_bytes: usize,
    /// Proposed bytes.
    pub after_bytes: usize,
    /// Added lines.
    pub added_lines: usize,
    /// Removed lines.
    pub removed_lines: usize,
    /// Literal unified diff, absent for binary files.
    pub diff: Option<String>,
    /// A saved review omitted some diff text.
    pub diff_truncated: bool,
}
/// Review lifecycle; only Pending accepts a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ProtectedChangeStatus {
    /// Waiting for its owner.
    Pending,
    /// Owner approved; filesystem application is in progress.
    Applying,
    /// Every file applied.
    Applied,
    /// Owner declined, or stopped the turn.
    Declined,
    /// A captured base changed; nothing applied.
    Stale,
    /// Application stopped; the applied prefix is explicit.
    Failed,
}
/// The failing path and plain-text explanation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProtectedChangeProblem {
    /// First failing relative path, when known.
    pub path: Option<String>,
    /// Error without complete file contents.
    pub message: String,
}
/// One session's captured review. Saved views cannot reconstruct or apply a proposal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProtectedChangeView {
    /// Random proposal identity.
    pub id: String,
    /// Owning session; a decision must name this session too.
    pub session_id: String,
    /// The proposing turn, retained so the review stays at its step after the turn finishes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub turn_id: Option<String>,
    /// Literal agent explanation.
    pub reason: String,
    /// Capture time in Unix milliseconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_at_ms: u64,
    /// Decision time in Unix milliseconds, absent while pending.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub decided_at_ms: Option<u64>,
    /// Current lifecycle state.
    pub status: ProtectedChangeStatus,
    /// Captured files in application order.
    pub files: Vec<ProtectedFileView>,
    /// Paths successfully applied, including a partial prefix on failure.
    pub applied: Vec<String>,
    /// A stale or failed decision's explanation.
    pub problem: Option<ProtectedChangeProblem>,
    /// Optional user feedback on a decline.
    pub note: Option<String>,
    /// Declined because the turn was stopped.
    pub stopped: bool,
}
