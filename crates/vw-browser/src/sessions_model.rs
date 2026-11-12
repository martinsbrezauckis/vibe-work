//! Browser activity and action-log data shapes. Depends on serde and delegation receipts; called by session state and the core API.
use serde::{Deserialize, Serialize};

/// Who ran an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Actor {
    /// The session's agent (`browser_act` through its credential).
    Agent,
    /// A Debug API client (acting as the user).
    DebugApi,
    /// The app's own window (the user).
    User,
}

/// How an action ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// Done.
    Done,
    /// Refused before anything happened (policy, stale ref, secret field, not actionable, stopped).
    Refused,
    /// Failed while running.
    Failed,
    /// Cancelled by the user's Stop.
    Stopped,
}

/// One line of the action log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Increasing per session.
    pub seq: u32,
    /// When it ended (ms since the Unix epoch).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub at_ms: u64,
    /// Who.
    pub by: Actor,
    /// The action (`click`, `navigate`, ...).
    pub action: String,
    /// What it acted on (role and name, or a URL; page text, at most 80 characters).
    pub target: String,
    /// How it ended.
    pub outcome: Outcome,
    /// A short result or reason (at most 160 characters).
    pub detail: String,
}

/// What the Browser tab shows about agent use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    /// The session.
    pub session_id: String,
    /// Window focus handle of the chat's current agent web page, supplied by core's page registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub window_id: Option<String>,
    /// The agent acted within the last 30 s or is acting now ("Agent is using the browser").
    pub agent_active: bool,
    /// An agent action is running now.
    pub in_flight: bool,
    /// One authorized native click is waiting for the user to bring this window to the front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pending_native_click: Option<String>,
    /// The user pressed Stop; agent actions are refused until Resume or the next turn.
    pub stopped: bool,
    /// The last agent action (ms since the epoch).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_agent_at_ms: Option<u64>,
    /// Recent actions, newest last.
    pub log: Vec<LogEntry>,
    /// Metadata-only receipts for acts authorized by browser permission or native confirmation.
    pub receipts: Vec<crate::delegation::DelegationReceipt>,
}
