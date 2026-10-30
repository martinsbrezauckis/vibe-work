//! The `sessionTools` command's request and answer types: export a session as Markdown, the temporary
//! sessions of this run, the Subagents setting for new chats, and a session's excerpt for a session mention.
//!
//! Depends on: serde, ts-rs (feature `ts`). Called by: `command`, `session_export`, `temporary`, the UI (generated
//! `sessions.ts`), the Debug API (`POST /commands`).
//! Invariants: wire form `{"op": "<camelCase>", ...}` with unknown fields refused.

use serde::{Deserialize, Serialize};

/// What to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SessionToolsRequest {
    /// The whole session as Markdown (`session_export.rs`). The window gives no destination (it saves through the
    /// app's native save dialog, the `export_session` app command); the Debug API may name an absolute
    /// `destination`, else the Markdown comes back in the answer.
    Export {
        /// The session.
        session_id: String,
        /// Where to write the file (Debug API only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        destination: Option<String>,
    },
    /// Starts a temporary session: `startSession` with the same `agentConfig` (through the gate), recorded as
    /// temporary before the answer (deleted when Vibe Work quits, or at the next start after a crash).
    StartTemporary {
        /// `AgentConfig` object (filtered by the gate).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        agent_config: Option<serde_json::Value>,
    },
    /// The temporary sessions that exist now.
    Temporary {},
    /// Which other process holds a session, read now (Vibe 2.26.1 lease holder; `null`: free, or held by Vibe Work).
    Holder {
        /// The session.
        session_id: String,
    },
    /// Saves "the agent may use subagents" for NEW chats. Vibe applies it when a chat is created, so the core
    /// writes it through a short-lived empty chat in the app's own work folder and deletes that chat again
    /// (`subagents_setting.rs`). Chats that are already open keep the value they started with. The Debug API may
    /// use it too: it is the `config/write` the gate already allows.
    SetSubagents {
        /// The new value.
        enabled: bool,
    },
    /// A bounded excerpt of a session for a session mention (`session_excerpt.rs`); reads, never takes it.
    Excerpt {
        /// The session.
        session_id: String,
    },
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionToolsResult {
    /// `export`.
    Exported {
        /// What was written.
        export: SessionExportView,
    },
    /// `startTemporary`: the new session (loaded like any started session; `sessionStarted` events follow).
    Started {
        /// Its id.
        session_id: String,
    },
    /// `holder`.
    Holder {
        /// The holding process's pid, when another process holds it.
        holder_pid: Option<u32>,
    },
    /// `temporary`.
    Temporary {
        /// The temporary sessions' ids.
        session_ids: Vec<String>,
    },
    /// `setSubagents`: the value is saved.
    SubagentsSet {
        /// The value a new chat starts with, as Vibe reported it through the short-lived chat; `null` when that
        /// read failed (the caller reads the settings again).
        enabled: Option<bool>,
        /// The short-lived chat when it could not be deleted, even after one retry. It stays in the list as a
        /// temporary chat and is deleted when Vibe Work quits; `null` when it is gone.
        leftover_session_id: Option<String>,
    },
    /// `excerpt`.
    Excerpt {
        /// The excerpt.
        excerpt: SessionExcerptView,
    },
}

/// A session's title, first message and last answer, each bounded, vault values withheld (`session_excerpt.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SessionExcerptView {
    /// The session's title (or its first-message preview, or the untitled-session label).
    pub title: String,
    /// The first message the person sent, when there is one.
    pub first_user: Option<String>,
    /// The last answer, when there is one.
    pub last_answer: Option<String>,
}

/// The answer of `exportSession`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SessionExportView {
    /// A file name to suggest (the title, made safe, with `.md`).
    pub name: String,
    /// Size of the Markdown in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    /// User and agent messages written.
    pub messages: u32,
    /// Tool steps summarised.
    pub steps: u32,
    /// File changes written as diffs.
    pub diffs: u32,
    /// Kinds of known secret values that were found and replaced (`mistralApiKey`, ...); empty when none.
    pub secrets_replaced: Vec<String>,
    /// The history was longer than the export reads (the oldest part is missing; the document says so too).
    pub cut: bool,
    /// Where the file was written (a destination was given or chosen).
    pub saved_to: Option<String>,
    /// The Markdown itself, when no destination was given.
    pub markdown: Option<String>,
}
