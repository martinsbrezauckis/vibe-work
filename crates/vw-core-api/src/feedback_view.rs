//! Feedback wire types, generated into TypeScript; the window edits prose, never diagnostic fields.
use serde::{Deserialize, Serialize};

/// The kind of feedback the person is sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum FeedbackKind {
    /// A behavior that did not work.
    Bug,
    /// A requested capability.
    Feature,
}
/// Plain text supplied deliberately; no conversation or files are attached automatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeedbackDraft {
    /// Bug or feature request.
    pub kind: FeedbackKind,
    /// Short summary, at most 160 UTF-8 bytes.
    pub title: String,
    /// Reproduction, expected and actual behavior, or the requested capability; at most 8000 bytes.
    pub description: String,
    /// Optional plain text remarks; at most 2000 bytes.
    pub remarks: String,
}
/// Minimal diagnostic facts produced only by the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeedbackSystem {
    /// App release, to match regressions.
    pub app_version: String,
    /// Running agent release, or unknown when no agent is running.
    pub vibe_version: String,
    /// OS family, to identify the native implementation.
    pub os: String,
    /// Native OS release or Linux distribution release, to reproduce platform issues.
    pub os_version: String,
    /// CPU architecture, to identify the executable build.
    pub arch: String,
}
/// Immutable preview and outbound payload. Its random id is also the relay's retry key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FeedbackReport {
    /// Random report identifier; never a device or session identifier.
    pub id: String,
    /// Redacted prose exactly as shown for review.
    pub draft: FeedbackDraft,
    /// Present only with the person's sharing choice.
    pub system: Option<FeedbackSystem>,
}
/// The feedback panel's current core-owned state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FeedbackView {
    /// An open draft, absent when the panel is closed.
    pub draft: Option<FeedbackDraft>,
    /// Ready for the separate Send click, absent before Review.
    pub review: Option<FeedbackReport>,
    /// Read-only facts the person may include.
    pub system: FeedbackSystem,
    /// Remembered sharing choice, off by default.
    pub share_system: bool,
    /// Whether the release has a relay URL configured.
    pub available: bool,
}
/// Only prose and consent enter from callers. Send references an already reviewed core payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FeedbackRequest {
    /// Read an open panel, including after a window reload.
    Read,
    /// Open the panel; refuses to overwrite an existing draft.
    Open {
        /// Prose to offer for editing.
        draft: FeedbackDraft,
    },
    /// Sanitize edited text and create the immutable preview.
    Review {
        /// Prose deliberately entered by the person.
        draft: FeedbackDraft,
    },
    /// Remember consent; changes the preview too.
    Share {
        /// Whether to include the displayed diagnostic facts.
        share_system: bool,
    },
    /// Send exactly one reviewed payload. Only an undriven window can call this.
    Send {
        /// Identifier from the review result.
        review_id: String,
    },
    /// Discard the open draft without any network call.
    Close,
}
