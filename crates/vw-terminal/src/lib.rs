//! Integrated terminals, one or more per session: a real shell in a PTY (`ConPTY` on
//! Windows) started in the session's folder, with its whole process tree contained and cleaned up
//! with the session, and its output kept in a bounded scrollback and streamed with flow control.
//!
//! Responsibilities:
//! - [`TerminalManager`]: the registry of every terminal by session. Opens (with a per-session cap),
//!   writes, resizes, restarts, closes; closes a session's terminals when the session is archived
//!   or deleted and every terminal at app quit ([`TerminalManager::close_all`]).
//! - Containment (`contain`): Unix shells are session leaders (portable-pty calls `setsid`), so
//!   closing kills every process of that session, background jobs included; Windows shells live
//!   in a kill-on-close Job Object.
//! - Output (`flow`, `ring`): a scrollback ring with monotonic byte offsets (readers ask "from
//!   offset N"), one attached viewer at a time with acknowledgement-based flow control, so a 50 MB
//!   `cat` blocks the program instead of flooding the window; with no viewer a terminal keeps
//!   running and buffers into the bounded scrollback.
//! - Windows (`conpty`): the pseudo console's startup cursor query is answered by the core, so a
//!   shell renders even when no view is attached.
//! - Environment (`env`): the user's own environment minus hijack variables, the Mistral key and
//!   Vibe Work's `VW_*` variables (part 2 refined); `PORT` from the session's port lease.
//!
//! Crate placement: its own crate, not a module
//! of vw-platform, because it is a feature subsystem, it needs no `unsafe`, and vw-gate (which
//! depends on vw-platform) should not carry a PTY stack.
//!
//! Depends on: portable-pty, nix (Unix), win32job (Windows), vw-supervisor (hijack rule, key name), tokio.
//! Called by: vw-core-api (commands, session lifecycle), tests.
//! Invariants:
//! - A terminal's working directory comes from the caller's record of the session, never from a
//!   client, and a missing folder is an error (no silent fallback to `$HOME`).
//! - No terminal outlives [`TerminalManager::close_all`] or the manager itself: dropping the last
//!   handle kills every tree without waiting.
//! - Nothing in this crate blocks an async runtime thread: PTY reads, writes and waits run on
//!   dedicated threads; async methods hand blocking work to `spawn_blocking`.

mod config;
pub mod conpty;
mod contain;
mod env;
mod error;
mod flow;
mod manager;
mod pty;
mod ring;
mod seams;
mod shell;
mod terminal;
mod text;
mod types;

pub use config::{ParentEnv, TerminalConfig};
pub use env::terminal_env;
pub use error::TerminalError;
pub use manager::TerminalManager;
pub use ring::Ring;
pub use seams::{NoLeases, NoTerminalEvents, OwnProcess, PortLeaseSource, TerminalEvents};
pub use shell::{ShellCommand, default_shell};
pub use text::strip_ansi;
pub use types::{
    AttachInfo, ExitInfo, OpenRequest, OutputRead, TerminalEvent, TerminalId, TerminalInfo,
    TerminalState, ViewerSink,
};

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "integrated terminals per session: PTY shells, containment, output";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
