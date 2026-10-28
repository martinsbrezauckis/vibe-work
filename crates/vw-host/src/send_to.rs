//! Settings > General > "Right-click menus" and the one-time question about them: the switch reads and changes
//! the system's "Send to Vibe Work" entries (`vw_platform::send_to`), and `settings.json` remembers that the
//! question was answered (`sendToAsked`).
//!
//! Depends on: vw-platform (`send_to`), [`crate::settings`]. Called by: [`crate::Host::send_to`] (the app's
//! `send_to` command).
//! Invariants: the entries change only on an explicit [`SendToRequest::Set`]; the question is offered once: any
//! answer, or the switch itself, ends it. On Windows the installer asks it, so the window asks only when the
//! installer did not (an update installed from inside the app).

use std::path::Path;

use serde::{Deserialize, Serialize};
use vw_core_api::{CoreError, ErrorCode};
use vw_platform::send_to::{SendToError, SendToStatus};

use crate::settings::HostSettings;

/// What the window asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum SendToRequest {
    /// The switch and whether to offer the question.
    Read,
    /// Turns the entries on or off (also answers the question).
    Set {
        /// On or off.
        on: bool,
    },
    /// "Not now": the question is not offered again.
    NotNow,
}

/// The switch as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(clippy::struct_excessive_bools)] // independent facts the window shows, not a state machine
#[serde(rename_all = "camelCase")]
pub struct SendToView {
    /// `windows`, `macos`, `linux` or `other` (the status line differs per system).
    pub system: &'static str,
    /// The system offers such entries.
    pub supported: bool,
    /// The entries are there.
    pub on: bool,
    /// Windows 11: the entry is under "Show more options".
    pub under_more_options: bool,
    /// Show the one-time question.
    pub offer: bool,
}

/// The system's menus (the real ones, or a stand-in in tests).
pub trait Menus {
    /// What the menus look like now.
    fn status(&self) -> SendToStatus;
    /// Writes the entries; all or nothing.
    ///
    /// # Errors
    /// The first write that failed.
    fn install(&self) -> Result<(), SendToError>;
    /// Removes the entries.
    ///
    /// # Errors
    /// One that could not be removed.
    fn remove(&self) -> Result<(), SendToError>;
}

/// The menus of this computer.
pub struct SystemMenus;

impl Menus for SystemMenus {
    fn status(&self) -> SendToStatus {
        vw_platform::send_to::status()
    }
    fn install(&self) -> Result<(), SendToError> {
        vw_platform::send_to::install()
    }
    fn remove(&self) -> Result<(), SendToError> {
        vw_platform::send_to::remove()
    }
}

const SYSTEM: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "linux") {
    "linux"
} else {
    "other"
};

/// Answers one request against `menus`, remembering answers in the settings file at `settings`.
///
/// # Errors
/// `internal` with the plain sentence the window shows: "Could not add the menu entry: {reason}. Nothing was
/// changed." (or "remove"), or when the answer cannot be saved.
pub fn apply(
    settings: &Path,
    request: SendToRequest,
    menus: &impl Menus,
) -> Result<SendToView, CoreError> {
    let answered = |settings_file: &Path| {
        let mut saved = HostSettings::load(settings_file);
        saved.send_to_asked = Some(true);
        saved.save(settings_file).map_err(|error| {
            CoreError::new(
                ErrorCode::Internal,
                format!("cannot save the setting: {error}"),
            )
        })
    };
    match request {
        SendToRequest::Read => {}
        SendToRequest::NotNow => answered(settings)?,
        SendToRequest::Set { on } => {
            let changed = if on { menus.install() } else { menus.remove() };
            changed.map_err(|error| {
                let verb = if on { "add" } else { "remove" };
                let tail = if on { " Nothing was changed." } else { "" };
                CoreError::new(
                    ErrorCode::Internal,
                    format!("Could not {verb} the menu entry: {error}.{tail}"),
                )
            })?;
            answered(settings)?;
        }
    }
    let status = menus.status();
    let asked = HostSettings::load(settings).send_to_asked == Some(true);
    Ok(SendToView {
        system: SYSTEM,
        supported: status.supported,
        on: status.on,
        under_more_options: status.under_more_options,
        offer: status.supported && !status.on && !asked && !status.installer_asked,
    })
}

impl crate::Host {
    /// The "Right-click menus" switch of this computer (blocking: files, the registry or `defaults`).
    ///
    /// # Errors
    /// See [`apply`].
    pub fn send_to(&self, request: SendToRequest) -> Result<SendToView, CoreError> {
        apply(&self.settings_path, request, &SystemMenus)
    }
}

#[cfg(test)]
#[path = "send_to_tests.rs"]
mod tests;
