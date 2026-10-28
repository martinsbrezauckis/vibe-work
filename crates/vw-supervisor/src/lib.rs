//! Finds, spawns, initializes, restarts and shuts down the `vibe-app-server` child process.
//!
//! Depends on: vw-rpc (the connection), vw-protocol (`initialize` types), `process-wrap`
//! (process group / job object), tokio.
//! Called by: the app (one [`Supervisor`] per app); later vw-gate/vw-session take connections from
//! it.
//! Invariants:
//! - The server gets a whitelisted environment, a configurable `VIBE_HOME` and the key only as
//!   `MISTRAL_API_KEY` from an injected [`SecretProvider`].
//! - A dead or wedged server is detected (vw-rpc rules), its whole process tree killed and
//!   reaped, and a new one started with exponential backoff and a give-up limit.
//! - After a restart, [`SupervisorState::Ready`] lists the sessions that need `session/resume`;
//!   [`Supervisor::resume`] reports progress while a resume takes its time.
//! - Clean shutdown closes stdin, waits, then kills; no zombies. On Windows the server lives in a
//!   kill-on-close job object, so it dies with this process even on a hard kill.
//! - One process per app, multiplexed: many isolated
//!   channels, one per active session, plus the lobby channel [`Supervisor::current`] returns.

mod backoff;
mod child;
mod config;
mod discovery;
mod env;
mod handshake;
mod launch;
mod python;
mod shell_env;
mod supervisor;

pub use backoff::Backoff;
pub use config::{Recovering, SupervisorConfig, SupervisorError, SupervisorState};
pub use discovery::{
    BINARY_NAME, Discovered, Discovery, DiscoveryError, FoundVia, canonical_executable,
    is_executable,
};
pub use env::{
    API_KEY_VAR, ApiKey, EnvError, EnvPlan, EnvSecretProvider, INHERITED, LOOPBACK_NO_PROXY,
    MCP_TOKEN_VAR, NoSecret, SecretError, SecretProvider, build_env, is_hijack_var,
};
pub use handshake::{default_initialize_params, handshake};
pub use launch::{CHILD_STOP_ADAPTER, adapter_python};
pub use python::server_python;
pub use supervisor::{OpenedChannel, Supervisor};
pub use vw_agent_tools::agent_env::isolate_credentials;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "vibe-app-server process supervision";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
