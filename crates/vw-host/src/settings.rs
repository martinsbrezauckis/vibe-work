//! The few switches the host must know before any UI exists, persisted as `settings.json` in the
//! app's data directory. Today: the Debug API switch (on in development builds, off in
//! user builds unless the user turns it on in Settings), the per-session terminal cap, and two
//! user-build switches: sharing the Vibe CLI's sessions and downloading updates by itself.
//!
//! Depends on: serde. Called by: [`crate::Host`] (reads at start, writes when the UI flips the
//! switch). Other settings live in the core and UI; this file only holds host startup settings.
//! Invariants:
//! - A missing, unreadable or corrupt file means "defaults" and is logged; it never stops the app.
//! - Writes go to a temporary file in the same folder and are renamed into place, so a crash
//!   leaves the old file or the new one, never half of one.
//! - The file holds no secret.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::warn;

/// Persisted host settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSettings {
    /// The user's Debug API switch; `None` when never set (the build default applies).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_api: Option<bool>,
    /// Most terminals one session may have open; `None` keeps the default (4). Applied at
    /// start, clamped to 1..=16. No Settings control yet: the file is the setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminals_per_session: Option<u16>,
    /// User builds: list and save sessions in the Vibe CLI's session
    /// folder so the app and the CLI see the same history. `None` = the default, **on**.
    /// Applied at start; no Settings control yet: the file is the setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_cli_sessions: Option<bool>,
    /// User builds that carry an update endpoint: download a newer version in the background
    /// as soon as it is found (`None` = on). Off = ask before downloading. Checks and the notification happen
    /// either way, and nothing is ever installed without the user's agreement. A build without an endpoint never
    /// checks, whatever this says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_download: Option<bool>,
}

/// The highest per-session terminal cap the setting may ask for.
pub const MAX_TERMINALS_PER_SESSION: u16 = 16;

impl HostSettings {
    /// The terminal cap to use: the setting clamped to 1..=[`MAX_TERMINALS_PER_SESSION`], if set.
    #[must_use]
    pub fn terminal_cap(&self) -> Option<usize> {
        self.terminals_per_session
            .map(|n| usize::from(n.clamp(1, MAX_TERMINALS_PER_SESSION)))
    }

    /// Whether to share the Vibe CLI's session folder (default on).
    #[must_use]
    pub fn share_cli_sessions(&self) -> bool {
        self.share_cli_sessions.unwrap_or(true)
    }

    /// Whether a found update is downloaded in the background (default on).
    #[must_use]
    pub fn auto_download(&self) -> bool {
        self.auto_download.unwrap_or(true)
    }
}

impl HostSettings {
    /// Reads `path`; defaults when the file is missing or cannot be used.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                warn!(path = %path.display(), %error, "settings file is not valid; using defaults");
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                warn!(path = %path.display(), %error, "settings file unreadable; using defaults");
                Self::default()
            }
        }
    }

    /// Writes the settings atomically (temp file, then rename), creating the folder if needed.
    ///
    /// # Errors
    /// The I/O error when the folder or file cannot be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let fields = serde_json::to_value(self).map_err(std::io::Error::other)?;
        let serde_json::Value::Object(mut fields) = fields else {
            return Err(std::io::Error::other("app settings must be an object"));
        };
        for key in [
            "debugApi",
            "terminalsPerSession",
            "shareCliSessions",
            "autoDownload",
        ] {
            fields.entry(key).or_insert(serde_json::Value::Null);
        }
        vw_core_api::write_app_settings(path, fields)
    }
}
