//! Value-free secret scan requests, Settings status and per-chat findings.
//! Depends on serde; used by the dispatcher, subscriptions and generated client types. No paths or ranges escape.

use serde::{Deserialize, Serialize};

/// App-owned scan switch, off until explicitly enabled. Redaction is always a person's Redact on one finding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretScanSettings {
    /// Scan session logs in the background.
    #[serde(default)]
    pub enabled: bool,
}

/// Scanner operations available equally to the UI and Debug API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SecretScanRequest {
    /// Current settings, status and retained findings.
    Read {},
    /// Persist the switch.
    Write {
        /// New switch.
        settings: SecretScanSettings,
    },
    /// Full background scan, regardless of incremental stamps.
    CheckNow {},
    /// Manual redaction of a retained finding, also allowed while scanning is off or the Vault is locked.
    Redact {
        /// Rust-owned finding authority.
        finding_id: String,
    },
    /// User input postpones startup and yields a running scan between files.
    Activity {},
}

/// Status shown below the Settings rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SecretScanStatus {
    /// Scanning is off.
    #[default]
    Off,
    /// Waiting for startup or Check now.
    Idle,
    /// Background scan in progress.
    Checking,
    /// No unlock prompt is raised.
    Locked,
    /// No Vault value meets the scan's length and strength requirements.
    NoSecrets,
    /// A completed scan, possibly with unreadable files.
    Checked,
    /// Folder resolution or traversal failed; no file content is included.
    Error,
}

/// A named match, grouped with other secrets in its chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SecretLeakName {
    /// Vault name, never its value.
    pub name: String,
    /// Website password.
    pub website_password: bool,
    /// Existing Vault login for Change password; absent on keys and older cached findings.
    #[serde(default)]
    pub login_id: Option<String>,
    /// Matching places.
    pub places: u32,
    /// Keys already removed can share a chat entry with a password that still offers Redact.
    #[serde(default)]
    pub state: SecretLeakState,
}

/// State of one retained bell finding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SecretLeakState {
    /// Offers Redact and Open chat.
    #[default]
    Found,
    /// Close this chat first, then Redact.
    ChatOpen,
    /// All selected matches were removed and rescanned.
    Removed,
    /// Some files changed or could not be replaced.
    Partial,
}

/// One security bell entry per chat, with Rust-owned action authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SecretLeakFinding {
    /// Opaque action handle; never a location.
    pub finding_id: String,
    /// Session to open.
    pub chat_id: String,
    /// Display title, scrubbed before publication.
    pub chat_title: String,
    /// Names and counts only.
    pub secrets: Vec<SecretLeakName>,
    /// Matching places.
    pub places: u32,
    /// Current action state.
    pub state: SecretLeakState,
    /// Files replaced, for partial redaction text.
    pub redacted_files: u32,
    /// Affected files, for partial redaction text.
    pub files: u32,
    /// Files changed since the finding; the approved partial text applies to these.
    #[serde(default)]
    pub changed_files: u32,
    /// Files whose format would be damaged by the edit.
    #[serde(default)]
    pub format_files: u32,
    /// Files the OS refused to replace.
    #[serde(default)]
    pub unreadable_files: u32,
    /// Redact can work: one of the chat's files is a log from before Vibe 3.0. Vibe 3.0 chats are sealed by
    /// digests, so their only remedy is deleting the chat (Redact is not offered there).
    #[serde(default)]
    pub redactable: bool,
}

/// The scanner's value-free view. Live reports and file stamps remain in Rust.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SecretScanView {
    /// Saved switch.
    pub settings: SecretScanSettings,
    /// Current status.
    pub status: SecretScanStatus,
    /// Last completion, Unix milliseconds; clients format in the user's locale.
    pub checked_ms: Option<i64>,
    /// Current matching places.
    pub places: u32,
    /// Chats containing current matches.
    pub chats: u32,
    /// Unreadable files, without paths or contents.
    pub failed_files: u32,
    /// Vault values excluded by the length/strength policy, independent of any log contents.
    #[serde(default)]
    pub skipped_secrets: u32,
    /// Retained per-chat entries, including completed redactions.
    pub findings: Vec<SecretLeakFinding>,
}
