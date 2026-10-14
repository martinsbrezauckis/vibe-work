//! Process and port monitor: which process listens on which port, which session it
//! belongs to and why, per-session port leases, a safe stop that can only hit what the session owns, and a
//! process ledger so nothing this app or the agent started is ever left without an owner.
//!
//! Entry point: [`ProcMon`] (scan, session views, leases, busy-port explanations, stop, ledger, startup
//! sweep, session end, a session id hand-off, quit). The pure parts are public for black-box tests: [`parse_proc_net_tcp`],
//! [`local_urls`], [`busy_port_line`], [`Attribution`], [`PortLeases`], [`explain`].
//!
//! Depends on: vw-platform (process identity, user-only files), serde, tokio (the tick); on Linux nothing
//! else (own `/proc` reader); on Windows/macOS `listeners` + `sysinfo` (library choice:).
//! Called by: vw-core-api.
//! Invariants:
//! - Nothing is ever stopped on a guess: stops need a fresh scan, a matching start token, the same OS user,
//!   and attribution to the asking session (or the ledger's own record for the startup sweep).
//! - Automatic stops (session end, quit, sweep) only touch `owned` processes; `agent` processes run until
//!   someone asks to stop them.
//! - Unknown or unreadable = "other app", never an error and never claimed.

mod attribution;
mod error;
mod explain;
mod handoff;
mod leases;
mod ledger;
mod left_running;
mod lifecycle;
mod monitor;
mod os;
mod procnet;
mod registry;
mod signal;
mod stop;
mod text;
mod types;
mod view;

pub use attribution::{
    Adopted, Attributed, AttributedListener, Attribution, BackgroundProcess, Confidence, Evidence,
    Inputs, Owner, SessionContext, normalize,
};
pub use error::{LeaseError, LedgerError, ScanError};
pub use explain::{PortExplanation, PortOption, explain};
pub use handoff::HandOffReport;
pub use leases::{
    DEFAULT_PER_SESSION, DEFAULT_RANGE, LeaseConfig, PortLeases, PortReservation, bind_refused,
};
pub use ledger::{LEDGER_VERSION, Ledger, LedgerEntry, LedgerFile};
pub use lifecycle::{SweepItem, SweepReport};
pub use monitor::{ProcMon, ProcMonConfig};
pub use os::{fill_cmdline, identity, is_alive, snapshot};
pub use procnet::{ProcNetListener, parse_proc_net_tcp};
pub use registry::{OwnProcess, OwnProcessRegistry, ProcessClass, ProcessGroup, RegistrationId};
pub use stop::{Authority, StopConfig, StopError, StopOutcome, StopRefusal, stop};
pub use text::{BusyLine, LocalUrl, busy_port_line, local_urls};
pub use types::{ProcessInfo, ProcessRef, SelfInfo, Snapshot, SocketListener, StartToken};
pub use view::{BackgroundView, ListenerView, ProcessView, SessionEndReport, SessionProcesses};

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "process and port monitor: attribution, leases, safe stop, ledger";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
