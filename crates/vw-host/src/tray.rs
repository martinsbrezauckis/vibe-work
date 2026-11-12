//! Window-close decisions and installation preferences. Depends on the shared settings writer;
//! the desktop wiring calls it before hiding. Explicit exit and restart always proceed.
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

/// Whether a native request may hide the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    /// The window's Close action.
    Close,
    /// Quit, session end, or an update restart.
    Exit,
}

/// True only for a close with both the preference and a reachable tray.
#[must_use]
pub fn hides(action: WindowAction, enabled: bool, icon: bool, host: bool) -> bool {
    action == WindowAction::Close && enabled && icon && host
}

/// A hidden window must come back when its tray host is lost.
#[must_use]
pub fn must_reveal(visible: bool, host: bool) -> bool {
    !visible && !host
}

/// The General switch and whether this run offers it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayView {
    /// Close keeps the application running (default on).
    pub enabled: bool,
    /// This run has an icon and a reachable host.
    pub available: bool,
}

/// Preferences saved independently of host startup settings, preserving every unrelated field.
pub struct TrayPreferences {
    path: PathBuf,
    enabled: bool,
    noticed: bool,
}

impl TrayPreferences {
    /// Missing settings keep close-to-tray on and the notice unseen.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let value = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null);
        Self {
            path: path.to_owned(),
            enabled: value
                .get("closeToTray")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            noticed: value
                .get("trayNoticeShown")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// The confirmed switch.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Save before applying; failed writes keep the confirmed value.
    /// # Errors
    /// The settings writer's I/O error.
    pub fn set_enabled(&mut self, enabled: bool) -> std::io::Result<()> {
        self.write("closeToTray", enabled)?;
        self.enabled = enabled;
        Ok(())
    }

    /// Claims the first actual hide, once per installation, even if notifications are disabled
    /// or the OS refuses. Save before handing the notice to the OS, so a restart never repeats it.
    /// # Errors
    /// The settings writer's I/O error; no notice may be shown then.
    pub fn first_hide(&mut self) -> std::io::Result<bool> {
        if self.noticed {
            return Ok(false);
        }
        self.write("trayNoticeShown", true)?;
        self.noticed = true;
        Ok(true)
    }

    fn write(&self, key: &str, value: bool) -> std::io::Result<()> {
        let mut fields = serde_json::Map::new();
        fields.insert(key.to_owned(), json!(value));
        vw_core_api::write_app_settings(&self.path, fields)
    }
}
