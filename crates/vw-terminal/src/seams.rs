//! Seams to blocks built in parallel: the port lease a terminal exports as `PORT`, the registry of
//! our own processes the process monitor reads, and where lifecycle events go.
//!
//! Depends on: `types`. Called by: `manager` (asks for leases, publishes events, lists processes),
//! the core (implements [`TerminalEvents`] and [`PortLeaseSource`] over `vw-procmon`, and registers each
//! shell with the monitor's registry from the lifecycle events).

use crate::types::TerminalEvent;

/// Hands a session's leased port to its terminals ("ports are leased, not guessed").
///
/// `vw-procmon` owns the leases; the core passes its per-session leases here. [`NoLeases`] (no `PORT`) is
/// what tests that do not care use. `port_for` may block (a lease scans the OS tables); the manager calls
/// it off the async runtime.
pub trait PortLeaseSource: Send + Sync {
    /// The port a new terminal of `session_id` exports as `PORT`, or `None` for no `PORT`.
    fn port_for(&self, session_id: &str) -> Option<u16>;
}

/// No leases: terminals get no `PORT`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLeases;

impl PortLeaseSource for NoLeases {
    fn port_for(&self, _session_id: &str) -> Option<u16> {
        None
    }
}

/// One process tree we started for a session, for the process monitor.
///
/// `TerminalManager::own_processes` lists these (diagnostics, tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnProcess {
    /// The session.
    pub session_id: String,
    /// The terminal.
    pub terminal_id: String,
    /// The shell's process id.
    pub pid: u32,
    /// Unix: the session id every process of the tree shares (equals `pid`, the shell is the
    /// session leader). `None` on Windows.
    pub unix_session: Option<u32>,
    /// Windows: the process ids currently in the terminal's Job Object. Empty on Unix.
    pub job_members: Vec<u32>,
}

/// Receives terminal lifecycle events (the core turns them into core events for both clients).
pub trait TerminalEvents: Send + Sync {
    /// Called once per event, from whichever thread caused it; must not block.
    fn publish(&self, event: TerminalEvent);
}

/// Drops every event (tests that do not care).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTerminalEvents;

impl TerminalEvents for NoTerminalEvents {
    fn publish(&self, _event: TerminalEvent) {}
}
