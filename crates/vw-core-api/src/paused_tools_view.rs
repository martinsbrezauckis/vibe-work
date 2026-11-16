//! The paused-tools contract for the UI and Debug API: which agent tools the person paused in a chat, whether the
//! agent is waiting on one, and the person's Pause / Resume operations.
//! Depends on serde; generated TypeScript mirrors these core-owned values.

use serde::{Deserialize, Serialize};

/// One paused tool of a chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PausedTool {
    /// The paused agent tool (`browser`).
    pub tool: String,
    /// Where the person paused it, for the chat line (for example "Chrome" or "the built-in browser").
    pub place: String,
    /// When the pause started (Unix milliseconds).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub since_ms: u64,
    /// True while an agent call of this tool waits for the person.
    pub waiting: bool,
}

/// A chat's paused tools (empty when nothing is paused).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PausedToolsView {
    /// The chat.
    pub session_id: String,
    /// Paused tools, by tool name.
    pub paused: Vec<PausedTool>,
}

/// The person's pause operations (UI and Debug API; the agent has no route here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PausedToolsRequest {
    /// Read the chat's paused tools.
    Read {
        /// The chat.
        session_id: String,
    },
    /// Pause the agent's browser access in the chat: its next browser call waits until Resume or Stop.
    Pause {
        /// The chat.
        session_id: String,
        /// Where the person pressed Pause, shown in the chat line (at most 60 characters).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        place: Option<String>,
    },
    /// Resume the agent's browser access in the chat: a waiting call runs.
    Resume {
        /// The chat.
        session_id: String,
    },
}
