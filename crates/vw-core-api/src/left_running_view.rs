//! The `processes` command's request and answer types: what deleted sessions left running,
//! the question right after a delete, and Stop.
//!
//! Depends on: serde, ts-rs (feature `ts`). Called by: `command` (the `Command::Processes` variant), `left_running`
//! (builds them), the UI (generated TypeScript in `status.ts`), the Debug API (`POST /commands`).
//! Invariants: a view never carries a process's arguments or working folder (the same rule as vw-procmon's
//! views); a process is named by an opaque `id` that the core parses back, so the start token never passes through
//! a JavaScript number (Windows tokens exceed 2^53).

use serde::{Deserialize, Serialize};

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProcessesRequest {
    /// The "Left running" list: agent processes whose session was deleted (this run, or an earlier one found by
    /// the startup sweep) and that still run.
    LeftRunning {},
    /// Right after `session/delete` succeeded: when the deleted session left processes running, the window asks
    /// "Also stop its N running processes?" in the native dialog and stops them on Continue. The Debug API is never
    /// asked (it acts as the user): it gets the list and stops what it wants with `stop`.
    AfterDelete {
        /// The deleted session.
        session_id: String,
    },
    /// Stops one listed process (the list's Stop). Only an `id` of the list is accepted.
    Stop {
        /// The process's `id` from the list.
        id: String,
    },
    /// The processes panel: a session's dev servers and long commands: Vibe's background processes
    /// and the processes the agent started with `process_start`, with their commands as the agent wrote them.
    Session {
        /// The session.
        session_id: String,
    },
    /// The latest output of one of the session's processes (`id` from `session`), at most 16 KB.
    Output {
        /// The session.
        session_id: String,
        /// The process's `id` from `session`.
        id: String,
    },
    /// Stops one of the session's processes (`id` from `session`): Vibe's `session/backgroundProcess/stop` for its
    /// own, the process monitor's proven stop for one the agent started.
    StopSession {
        /// The session.
        session_id: String,
        /// The process's `id` from `session`.
        id: String,
    },
}

/// One process of a session in the processes panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SessionProcessView {
    /// Opaque handle for `output` and `stopSession` (`vibe:<processId>` or `started:<pid>-<start token>`).
    pub id: String,
    /// `vibe` (Vibe's background process: it ends when Vibe or the session's channel ends) or `started` (started
    /// for the agent by Vibe Work: it keeps running until stopped, also across quit).
    pub kind: String,
    /// The command as the agent wrote it, when known.
    pub command: Option<String>,
    /// `running`, `exited`, `orphaned`, ... (Vibe's word for its own; `running` for a live started one).
    pub status: String,
    /// Pid (started ones).
    pub pid: Option<u32>,
    /// Ports it listens on (started ones; Vibe does not report its own).
    pub ports: Vec<u16>,
    /// Re-attached after an app restart ("still running from last time").
    pub earlier: bool,
}

/// One process a deleted session left running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LeftRunningView {
    /// Opaque handle for `stop` (`<pid>-<start token>`).
    pub id: String,
    /// The deleted session it belonged to.
    pub session_id: String,
    /// That session's title when it was deleted (sanitized), if one was known.
    pub session_title: Option<String>,
    /// Pid.
    pub pid: u32,
    /// Executable name (never from the command line).
    pub name: Option<String>,
    /// Ports it listened on when last seen.
    pub ports: Vec<u16>,
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProcessesResult {
    /// `leftRunning`.
    LeftRunning {
        /// Oldest first.
        items: Vec<LeftRunningView>,
    },
    /// `afterDelete`.
    AfterDelete {
        /// The native dialog was shown (false: nothing was left running, or the Debug API asked).
        asked: bool,
        /// Stopped on Continue.
        stopped: Vec<LeftRunningView>,
        /// Processes that could not be stopped, with the reason.
        failed: Vec<String>,
        /// The session's processes still running now (Cancel, a failure, or the Debug API's answer).
        left: Vec<LeftRunningView>,
    },
    /// `session`.
    Session {
        /// The session's processes, running ones first.
        items: Vec<SessionProcessView>,
    },
    /// `output`.
    Output {
        /// The process.
        id: String,
        /// The latest output (at most 16 KB; may start mid-line).
        text: String,
        /// False when no output can be read (Vibe has none for it, or the log is gone).
        available: bool,
    },
    /// `stopSession`.
    StoppedSession {
        /// The process.
        id: String,
    },
    /// `stop`.
    Stopped {
        /// The process.
        item: LeftRunningView,
        /// It needed KILL after TERM.
        escalated: bool,
    },
}
