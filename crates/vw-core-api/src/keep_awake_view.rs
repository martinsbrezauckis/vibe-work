//! Shared idle-sleep settings and status contract for the UI and Debug API.
//! Depends on serde; generated TypeScript mirrors these core-owned values.

use serde::{Deserialize, Serialize};

/// Whether the app owns an idle-sleep request, or the OS refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum KeepAwakeStatus {
    /// No request is held (idle or the setting is off).
    Idle,
    /// The OS accepted the idle-sleep request.
    Active,
    /// The OS refused; no automatic retry while this work continues.
    Refused,
}

/// The saved preference and the live, app-wide hold status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KeepAwakeView {
    /// Keep this computer awake while Vibe is working. On by default.
    pub enabled: bool,
    /// Current OS request status.
    pub status: KeepAwakeStatus,
}

/// Operations shared by Settings and the Debug API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum KeepAwakeRequest {
    /// Read the preference and live status, without contacting the OS.
    Read {},
    /// Persist the preference; turning it off releases a live request before returning.
    Write {
        /// New preference.
        enabled: bool,
    },
}
