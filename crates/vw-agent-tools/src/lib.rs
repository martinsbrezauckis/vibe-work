//! Session-scoped agent tools: what the agent of one session may do with its
//! processes and ports, and the guard on raw `kill` commands.
//!
//! Pieces, each usable on its own:
//! - [`wire`]: the one-request-per-connection NDJSON format between the `vw-agent-tools` binary (MCP server or
//!   hook) and the Vibe Work core, plus the environment names and limits both sides share.
//! - [`credential`]: the core's table of per-session random credentials (constant-time lookup).
//! - [`killparse`]: literal starts of `kill` / `pkill` / `killall` / `taskkill` / `Stop-Process` / `spps`.
//! - [`mcp`]: the stdio MCP server loop (answers `initialize` / `tools/list` itself, forwards `tools/call`).
//! - [`gitparse`]: literal pushes/forge writes and credential commands refused; branch rewrites outside
//!   the session's copy branch asked, ordinary local git left to Vibe's card.
//! - [`agent_env`]: one credential policy for every agent-capable process environment.
//! - [`hook`]: the `pre_tool` guards Vibe runs from `$VIBE_HOME/hooks.toml` (kill, git, folder and connector guards,
//!   safety reminders).
//! - [`folders`]: the folder guard's rule: another session's copy is not this session's to touch.
//! - [`forge`]: the connector rule: connector tools that write to a code forge are the user's button.
//! - [`safety`]: the safety reminders' rules and settings.
//! - [`secretspec`]: the `secret_run` request (program, arguments, secrets into variables) and its checks.
//! - [`spawn`] and [`output`]: how the core starts an agent process that must outlive the app (own process
//!   group / job, output to a file) and reads its output back.
//!
//! Depends on: serde, `serde_json`, getrandom, subtle (no async runtime: the binary starts once per bash call).
//! Called by: the `vw-agent-tools` binary (`main.rs`), vw-core-api (`agent_tools`), tests.
//! Invariants:
//! - A credential travels only in the MCP server's environment and in the request line to the core; never in
//!   argv, a log line or an error text.
//! - Nothing here decides ownership: the core does, from the process monitor (vw-procmon).

pub mod agent_env;
pub mod cli;
mod command_segments;
pub mod credential;
pub mod folders;
pub mod forge;
pub mod gitparse;
mod gitparse_cmds;
pub mod hook;
pub mod killparse;
mod killparse_cmds;
mod killparse_shell;
pub mod mcp;
pub mod output;
pub mod safety;
pub mod secretspec;
pub mod spawn;
mod specs;
pub mod wire;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics can assert
/// which crate they are looking at.
pub const CRATE_ROLE: &str =
    "session-scoped agent tools: MCP server, kill guard, credentials, agent spawn";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
