//! The processes panel's view of the processes the agent started with `process_start`: which
//! of a session's are alive, their command (as the agent wrote it, already in its transcript), ports, and the tail
//! of their output log.
//!
//! Depends on: `launches`, `start` (`LABEL`), `session` (`alive`, `tree_listeners`), vw-procmon (scan, registry,
//! re-attached processes), vw-agent-tools (`output::tail`). Called by: `session_processes` (the `processes` command's
//! `session` and `output` ops).
//! Invariants: read-only (dead entries are left for `processes_list` to clean); blocking (the caller uses
//! `spawn_blocking`).

use vw_procmon::ProcessRef;

use super::AgentTools;
use super::session::{alive, tree_listeners};
use super::start::LABEL;
use crate::core::Shared;

/// Most bytes of a started process's output shown.
pub(crate) const TAIL: usize = 16 * 1024;

/// One started process, as the panel lists it.
pub(crate) struct Started {
    pub(crate) process: ProcessRef,
    pub(crate) command: Option<String>,
    pub(crate) ports: Vec<u16>,
    /// Re-attached after an app restart ("still running from last time").
    pub(crate) earlier: bool,
}

/// The session's live `process_start` processes.
pub(crate) fn started(shared: &Shared, tools: &AgentTools, session_id: &str) -> Vec<Started> {
    let Ok(snapshot) = shared.procmon.scan() else {
        return Vec::new();
    };
    let sessions = crate::processes::contexts(&shared.store);
    let inputs = shared.procmon.inputs(&sessions);
    let mut ours: Vec<(ProcessRef, bool)> = shared
        .procmon
        .registry()
        .for_session(session_id)
        .into_iter()
        .filter(|p| p.label == LABEL)
        .map(|p| (p.process, false))
        .collect();
    for adopted in inputs.adopted.iter().filter(|a| a.session_id == session_id) {
        if !ours.iter().any(|(r, _)| *r == adopted.process) {
            ours.push((adopted.process, true));
        }
    }
    ours.into_iter()
        .filter(|(process, _)| alive(&snapshot, *process))
        .map(|(process, earlier)| Started {
            process,
            command: tools.launches.of(process).map(|l| l.command),
            ports: tree_listeners(&snapshot, process)
                .into_iter()
                .map(|(_, port)| port)
                .collect(),
            earlier,
        })
        .collect()
}

/// The tail of a started process's output log, when it has one.
pub(crate) fn tail(tools: &AgentTools, process: ProcessRef) -> Option<String> {
    let launch = tools.launches.of(process)?;
    vw_agent_tools::output::tail(&launch.log, TAIL).ok()
}
