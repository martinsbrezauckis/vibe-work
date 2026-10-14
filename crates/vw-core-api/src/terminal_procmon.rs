//! Terminals <-> process monitor: the session's leased port becomes a terminal's `PORT`, and
//! every terminal shell is registered with the monitor as an `owned` process of its session.
//!
//! Depends on: vw-procmon (`ProcMon`, `OwnProcessRegistry`), vw-terminal (seams, events). Called by:
//! `core` (construction), `terminals` (`EventBridge` forwards every lifecycle event here).
//! Invariants:
//! - Registration follows the lifecycle events, which carry the shell pid: `opened` and `restarted`
//!   register (replacing the terminal's earlier entry), `exited` and `closed` unregister. The monitor sees
//!   the shell as a fact (identity = pid + start token), and servers started in it as its descendants.
//! - An `exited` event of an older shell never removes a newer one (the pid must match).
//! - A shell whose identity cannot be read (already gone) is not registered.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use tracing::debug;
use vw_procmon::{
    OwnProcess, OwnProcessRegistry, ProcMon, ProcessClass, ProcessGroup, RegistrationId, identity,
};
use vw_terminal::{PortLeaseSource, TerminalEvent, TerminalInfo};

/// Label shown for a registered terminal shell.
const LABEL: &str = "terminal";

/// The session's lease as `PORT`: the first leased port, leasing on the first terminal of a session.
pub(crate) struct MonitorLeases(pub(crate) ProcMon);

impl PortLeaseSource for MonitorLeases {
    /// Blocking (the first lease of a session scans the OS tables). `None` when the range has no free port.
    fn port_for(&self, session_id: &str) -> Option<u16> {
        if let Some(port) = self.0.leases().of(session_id).first() {
            return Some(*port);
        }
        // A partial lease (range nearly full) is kept by the monitor, so the first port is read back.
        let _ = self.0.lease(session_id);
        self.0.leases().of(session_id).first().copied()
    }
}

/// Keeps the monitor's registry equal to the set of running terminal shells.
pub(crate) struct ShellRegistrar {
    registry: OwnProcessRegistry,
    /// Terminal id -> (shell pid, registration).
    held: Mutex<HashMap<String, (u32, RegistrationId)>>,
}

impl ShellRegistrar {
    pub(crate) fn new(registry: OwnProcessRegistry) -> Self {
        Self {
            registry,
            held: Mutex::new(HashMap::new()),
        }
    }

    /// Applies one terminal lifecycle event. Called from whichever thread caused it; never blocks long.
    pub(crate) fn apply(&self, event: &TerminalEvent) {
        match event {
            TerminalEvent::Opened(info) | TerminalEvent::Restarted(info) => self.register(info),
            TerminalEvent::Exited(info) => self.unregister(&info.id.to_string(), info.pid),
            TerminalEvent::Closed { id, .. } => self.unregister(&id.to_string(), None),
        }
    }

    fn register(&self, info: &TerminalInfo) {
        let terminal = info.id.to_string();
        self.unregister(&terminal, None);
        let Some(pid) = info.pid else { return };
        let Some(process) = identity(pid) else {
            debug!(terminal, pid, "terminal shell already gone, not registered");
            return;
        };
        let group = if cfg!(windows) {
            ProcessGroup::WindowsJob {
                name: format!("terminal-{terminal}"),
            }
        } else {
            // The shell is a session leader (`setsid`), so its group id is its pid.
            ProcessGroup::Unix { pgid: pid }
        };
        let id = self.registry.register(OwnProcess {
            session_id: Some(info.session_id.clone()),
            process,
            group,
            class: ProcessClass::Owned,
            label: LABEL.to_owned(),
            keep_running: false,
        });
        self.lock().insert(terminal, (pid, id));
    }

    /// Removes the terminal's registration; with `only_pid`, only when it still names that shell.
    fn unregister(&self, terminal: &str, only_pid: Option<u32>) {
        let mut held = self.lock();
        if let Some((pid, _)) = held.get(terminal)
            && only_pid.is_some_and(|wanted| wanted != *pid)
        {
            return;
        }
        if let Some((_, id)) = held.remove(terminal) {
            let _ = self.registry.unregister(id);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (u32, RegistrationId)>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
