//! A terminal's environment. A user's terminal is their own shell, so it starts from the parent environment and only removes what must never
//! reach it; the allowlist (`vw_supervisor::INHERITED`) stays for `vibe-app-server`, MCP servers and
//! helpers, which run the agent's commands.
//!
//! Depends on: vw-supervisor (`is_hijack_var`, `API_KEY_VAR`), vw-platform (`child_env`). Called by: `manager` (each open and
//! restart).
//! Removed from the parent environment:
//! - loader and runtime hijack variables (`is_hijack_var`: `LD_*`, `DYLD_*`, `BASH_ENV`, `ENV`,
//!   `PYTHON*`, `NODE_OPTIONS`, `GIT_*`,...): set in the app's own environment they would run code
//!   in every shell before the user's rc files;
//! - `MISTRAL_API_KEY`, whatever its value (the real key lives only in the core);
//! - Vibe Work's own variables (`VW_*`: development, test and internal settings such as data folders
//!   and key handles), so nothing of the app's internals leaks into a user's shell;
//! - an inherited `PORT` (the app's own `PORT` would point at the wrong thing); `PORT` is set only
//!   from the session's lease.
//!
//! - what Vibe Work set for its own run only (`vw_platform::child_env`: the accessibility switch it may turn off
//!   on Linux); a `NO_AT_BRIDGE` the user set themselves stays.
//!
//! Other ordinary variables pass through unchanged (`DISPLAY`, `JAVA_HOME`, `EDITOR`,...),
//! and the user's rc files still run (login shell on Unix, the PowerShell profile on Windows).
//! portable-pty's builder starts from the whole process environment (and on Windows merges the
//! registry environment); `pty::spawn` clears it, so this list is exactly what the shell gets.
//!
//! The terminal's general environment filter lives here; the shared agent credential policy
//! runs last, including for terminals that can execute agent-supplied commands.

use std::ffi::{OsStr, OsString};

use vw_supervisor::{API_KEY_VAR, is_hijack_var};

/// `TERM_PROGRAM` value, so scripts can tell they run inside the app's terminal.
const TERM_PROGRAM: &str = "vibe-work";

/// Prefix of Vibe Work's own environment variables.
const INTERNAL_PREFIX: &str = "VW_";

/// Builds the environment for a terminal from `parent` (normally the process environment) and the
/// session's leased port.
#[must_use]
pub fn terminal_env(
    parent: &[(OsString, OsString)],
    port: Option<u16>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> =
        vw_platform::child_env::without_own_run_settings(parent.to_vec())
            .into_iter()
            .filter(|(name, _)| passes(name))
            .collect();
    let mut set = |name: &str, value: OsString| {
        env.retain(|(existing, _)| !same_name(existing, OsStr::new(name)));
        env.push((OsString::from(name), value));
    };
    // What xterm.js implements (256 colours, truecolor), as ShellX set it (terminal.rs:461-464).
    set("TERM", OsString::from("xterm-256color"));
    set("COLORTERM", OsString::from("truecolor"));
    set("TERM_PROGRAM", OsString::from(TERM_PROGRAM));
    if let Some(port) = port {
        set("PORT", OsString::from(port.to_string()));
    }
    vw_supervisor::isolate_credentials(&mut env);
    env
}

/// True when a parent variable may reach the shell.
fn passes(name: &OsStr) -> bool {
    // A name that is not Unicode cannot be checked against the rules: dropped.
    let Some(name) = name.to_str() else {
        return false;
    };
    let upper = name.to_ascii_uppercase();
    !is_hijack_var(name)
        && upper != API_KEY_VAR
        && upper != "PORT"
        && !upper.starts_with(INTERNAL_PREFIX)
}

/// Windows environment names are case-insensitive; Unix names are exact.
fn same_name(a: &OsStr, b: &OsStr) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}
