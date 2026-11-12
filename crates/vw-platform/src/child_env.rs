//! What programs Vibe Work starts must not inherit from settings it made for its own run. Today that is one
//! setting: on Linux the startup accessibility check ([`crate::a11y_bus`]) may set `NO_AT_BRIDGE=1` for this
//! run only. A shell, the file manager or a browser started from the app would otherwise run with
//! accessibility off too. The check sets [`BRIDGE_SET_BY_APP_VAR`] next to it, so a `NO_AT_BRIDGE` the user
//! set themselves (the check then changes nothing) still reaches the programs they start.
//!
//! Depends on: std. Called by: `a11y_bus` (sets the marker), the openers here (`open_url`, `open_file`,
//! `reveal`), vw-terminal (a terminal's environment).

use std::ffi::OsString;
use std::process::Command;

/// The accessibility bridge switch read by GTK, the web view and other programs.
pub const BRIDGE_VAR: &str = "NO_AT_BRIDGE";

/// Set next to [`BRIDGE_VAR`] when this run set the switch itself.
pub const BRIDGE_SET_BY_APP_VAR: &str = "VW_NO_AT_BRIDGE_SET";

/// `vars` (normally this process's environment) without the settings made for this run only: `NO_AT_BRIDGE`
/// and its marker when the marker is there; unchanged otherwise.
#[must_use]
pub fn without_own_run_settings(mut vars: Vec<(OsString, OsString)>) -> Vec<(OsString, OsString)> {
    if vars.iter().any(|(name, _)| name == BRIDGE_SET_BY_APP_VAR) {
        vars.retain(|(name, _)| name != BRIDGE_VAR && name != BRIDGE_SET_BY_APP_VAR);
    }
    vars
}

/// Removes the settings made for this run only from what `command` inherits from this process.
pub fn remove_own_run_settings(command: &mut Command) {
    if std::env::var_os(BRIDGE_SET_BY_APP_VAR).is_some() {
        command
            .env_remove(BRIDGE_VAR)
            .env_remove(BRIDGE_SET_BY_APP_VAR);
    }
}
