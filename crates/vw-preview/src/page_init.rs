//! The browser page environment: an initialization script every page of a session's browser gets at
//! document start, next to the console script. It answers dialogs raised during an agent action, and counts DOM mutations for settling (`js/page_init.js`).
//!
//! Depends on: `console` (the per-mount accessor), `serde_json`. Called by: `service` (mount), vw-browser (the
//! accessor name in its scripts), tests.
//! Invariants: the script installs one non-enumerable, non-configurable object under [`page_accessor`]; it never
//! sends anything anywhere (no network, no IPC).

/// The script source, `__VW_NAME__` replaced at mount.
const SOURCE: &str = include_str!("js/page_init.js");

/// The page environment's accessor name for a mount whose console accessor is `console_accessor`.
#[must_use]
pub fn page_accessor(console_accessor: &str) -> String {
    format!("{console_accessor}_page")
}

/// The initialization script for the accessor `name`.
#[must_use]
pub fn page_init_script(name: &str) -> String {
    SOURCE.replace(
        "__VW_NAME__",
        &serde_json::Value::String(name.to_owned()).to_string(),
    )
}
