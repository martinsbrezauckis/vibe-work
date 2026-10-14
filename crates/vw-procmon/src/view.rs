//! Serializable views the core and the Debug API return (camelCase JSON). Built from a snapshot plus its
//! attribution; nothing here reads the OS.
//!
//! Depends on: `attribution`, `ledger`, `registry`, `stop`, `types`, serde. Called by: `monitor`,
//! `lifecycle`, `explain`, vw-core-api, vw-debug-api.
//! Invariants:
//! - **No view carries process arguments**, ever: a denylist redaction of free-form argv cannot be made
//!   complete (and tokenizes differently from the program it describes). A view shows the executable name
//!   (basename of the path the OS reports; never argv[0] or `comm`, which the process controls), the pid,
//!   the start token, the argument COUNT, ports, and
//!   the cwd only for a process proven to be the session's (`confidence = fact`). The full command line is
//!   only available to the app window through `ProcMon::command_line` (fact-owned only), never here.
//! - Another app's process, or one only inferred to be a session's, shows name and pid only.

use serde::{Deserialize, Serialize};

use crate::attribution::{Attributed, AttributedListener, Confidence, Evidence, Owner};
use crate::ledger::LedgerEntry;
use crate::registry::{OwnProcess, ProcessClass};
use crate::stop::StopOutcome;
use crate::types::{ProcessInfo, Snapshot, StartToken};

/// One process for display. Never holds arguments (see the module invariants).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessView {
    /// Pid.
    pub pid: u32,
    /// Start token (proven processes only; pass it back to stop this exact process).
    pub start: Option<StartToken>,
    /// Executable name: basename of the OS-reported executable path; `None` when unreadable. Never taken
    /// from the command line (argv[0] is under the process's control).
    pub name: Option<String>,
    /// Parent pid (proven processes only).
    pub ppid: Option<u32>,
    /// Process group (proven processes only, Unix).
    pub pgid: Option<u32>,
    /// Number of arguments after argv[0] (proven processes only; the arguments themselves are never shown).
    pub arg_count: Option<usize>,
    /// Working directory (proven processes only).
    pub cwd: Option<String>,
    /// Ports it listens on (proven processes only; a listener view carries its own port anyway).
    pub ports: Vec<u16>,
    /// Who started it, when proven.
    pub class: Option<ProcessClass>,
}

impl ProcessView {
    /// View of a process proven to be a session's.
    #[must_use]
    pub fn ours(info: &ProcessInfo, snapshot: &Snapshot, class: Option<ProcessClass>) -> Self {
        Self {
            pid: info.pid,
            start: info.start,
            name: info.exe_name(),
            ppid: info.ppid,
            pgid: info.pgid,
            arg_count: (!info.cmdline.is_empty()).then(|| info.cmdline.len() - 1),
            cwd: info.cwd.as_ref().map(|c| c.display().to_string()),
            ports: snapshot.ports_of(info.pid),
            class,
        }
    }

    /// The view an attribution allows: [`ProcessView::ours`] only for a session owner proven as a fact,
    /// otherwise [`ProcessView::foreign`].
    #[must_use]
    pub fn for_attributed(
        info: &ProcessInfo,
        snapshot: &Snapshot,
        attributed: &Attributed,
    ) -> Self {
        if attributed.confidence == Some(Confidence::Fact) {
            Self::ours(info, snapshot, attributed.provenance)
        } else {
            Self::foreign(info)
        }
    }

    /// Name and pid only: another app's process, or one only inferred to be a session's.
    #[must_use]
    pub fn foreign(info: &ProcessInfo) -> Self {
        Self {
            pid: info.pid,
            start: None,
            name: info.exe_name(),
            ppid: None,
            pgid: None,
            arg_count: None,
            cwd: None,
            ports: Vec::new(),
            class: None,
        }
    }

    /// View of a ledger entry (no live process data).
    #[must_use]
    pub fn from_ledger(entry: &LedgerEntry) -> Self {
        Self {
            pid: entry.process.pid,
            start: Some(entry.process.start),
            name: entry.name.clone(),
            ppid: None,
            pgid: match entry.group {
                crate::registry::ProcessGroup::Unix { pgid } => Some(pgid),
                _ => None,
            },
            arg_count: Some(entry.arg_count),
            cwd: None,
            ports: entry.ports.clone(),
            class: Some(entry.class),
        }
    }
}

/// One listening socket with its owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenerView {
    /// Local address.
    pub address: String,
    /// Port.
    pub port: u16,
    /// Owner.
    pub owner: Owner,
    /// Evidence.
    pub evidence: Vec<Evidence>,
    /// The process (full for ours, name and pid for others).
    pub process: Option<ProcessView>,
}

impl ListenerView {
    /// Builds the view of one attributed listener.
    #[must_use]
    pub fn new(item: &AttributedListener, snapshot: &Snapshot) -> Self {
        let process = item
            .listener
            .pid
            .and_then(|pid| snapshot.processes.get(&pid))
            .map(|info| ProcessView::for_attributed(info, snapshot, &item.attributed));
        Self {
            address: item.listener.address.to_string(),
            port: item.listener.port,
            owner: item.attributed.owner.clone(),
            evidence: item.attributed.evidence.clone(),
            process,
        }
    }
}

/// A Vibe background process as session state shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundView {
    /// Vibe's `processId`.
    pub process_id: String,
    /// Status. (No command: views never carry arguments; Vibe's own session state is a separate route.)
    pub status: String,
}

/// Everything about one session's processes and ports (Debug API `GET /sessions/{id}/processes`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProcesses {
    /// Session id.
    pub session_id: String,
    /// Listeners attributed to the session.
    pub listeners: Vec<ListenerView>,
    /// Listeners where this session is only a candidate (ambiguous or URL-only): "other app".
    pub contested: Vec<ListenerView>,
    /// Processes we registered for the session (terminals, helpers).
    pub own: Vec<OwnProcess>,
    /// Agent processes re-attached after a restart ("still running from last time").
    pub reattached: Vec<ProcessView>,
    /// Vibe's background processes for the session (from session state; no pids there).
    pub background: Vec<BackgroundView>,
    /// The session's leased ports.
    pub leases: Vec<u16>,
    /// Scan duration in microseconds.
    pub scan_micros: u64,
}

/// What ending a session did (archive, delete).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEndReport {
    /// Session id.
    pub session_id: String,
    /// `owned` processes stopped.
    pub stopped: Vec<StopOutcome>,
    /// `owned` processes that could not be stopped, with the reason.
    pub failed: Vec<String>,
    /// `agent` processes still running, left alone (the UI asks "Also stop its N running processes?").
    pub agent_running: Vec<ProcessView>,
    /// Ports the stopped processes held that are STILL listened on afterwards (should be empty).
    pub leftover_ports: Vec<u16>,
    /// Leased ports returned.
    pub released_ports: Vec<u16>,
}
