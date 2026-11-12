//! Windows and macOS scan: IP Helper listener-only tables on Windows, `listeners` on macOS; `sysinfo` for the
//! process table (command line, cwd, parent, owner), and `vw-platform::process` for what `sysinfo` lacks or
//! rounds: the precise start token, and on macOS the process group and owner uid.
//!
//! Depends on: listeners 0.6.1 (macOS), sysinfo 0.38.4, vw-platform (TCP tables and identity), `types`. Called by:
//! `os::snapshot`, `os::is_alive`. Native execution requires Windows/macOS; the Windows table decoder
//! also has synthetic tests that run on Linux.
//! Invariants:
//! - A process `sysinfo` cannot open has `same_user = None` (other users' processes, elevated ones).
//! - macOS: the command line is the kernel's exact argv ([`vw_platform::process_argv`]): `sysinfo` drops empty
//!   arguments and then reads environment strings as arguments (a retitled process showed its environment).
//! - macOS: listeners of other users are invisible (libproc denies their descriptors without root); the
//!   lease bind test still sees their ports as taken.

use std::collections::BTreeMap;
use std::time::Instant;

#[cfg(target_os = "macos")]
use listeners::{Protocol, SocketState};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

use crate::error::ScanError;
use crate::types::{ProcessInfo, ProcessRef, SelfInfo, Snapshot, SocketListener, StartToken};

/// True when `process` still runs with the same start token (and is not a zombie on macOS).
pub(crate) fn is_alive(process: ProcessRef) -> bool {
    vw_platform::process_facts(process.pid)
        .is_some_and(|f| f.start_token == process.start.0 && !f.zombie)
}

/// One full scan.
pub(crate) fn snapshot() -> Result<Snapshot, ScanError> {
    let started = Instant::now();
    #[cfg(target_os = "macos")]
    let listeners: Vec<SocketListener> = listeners::get_all()
        .map_err(|e| ScanError::Unreadable(format!("socket table: {e}")))?
        .into_iter()
        .filter(|l| l.protocol == Protocol::TCP && l.state == SocketState::Listen)
        .map(|l| SocketListener {
            address: l.socket.ip(),
            port: l.socket.port(),
            pid: Some(l.process.pid),
            same_user: None,
        })
        .collect();
    #[cfg(windows)]
    let listeners: Vec<SocketListener> = vw_platform::windows_tcp_listeners()
        .map_err(|e| ScanError::Unreadable(format!("socket table: {e}")))?
        .into_iter()
        .map(|l| SocketListener {
            address: l.address,
            port: l.port,
            pid: Some(l.pid),
            same_user: None,
        })
        .collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .with_user(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    let my_pid = std::process::id();
    let my_user = system
        .process(Pid::from_u32(my_pid))
        .and_then(|p| p.user_id().cloned());
    let my_facts = vw_platform::process_facts(my_pid);
    let mut processes = BTreeMap::new();
    for (pid, process) in system.processes() {
        let pid = pid.as_u32();
        let facts = vw_platform::process_facts(pid);
        // macOS: the BSD info's uid is authoritative; Windows: compare the token owner SIDs sysinfo read.
        let same_user = match (facts.and_then(|f| f.uid), my_facts.and_then(|f| f.uid)) {
            (Some(theirs), Some(mine)) => Some(theirs == mine),
            _ => match (process.user_id(), my_user.as_ref()) {
                (Some(theirs), Some(mine)) => Some(theirs == mine),
                _ => None,
            },
        };
        let alive = !matches!(
            process.status(),
            ProcessStatus::Zombie | ProcessStatus::Dead
        ) && !facts.is_some_and(|f| f.zombie);
        processes.insert(
            pid,
            ProcessInfo {
                pid,
                ppid: process.parent().map(Pid::as_u32),
                pgid: facts.and_then(|f| f.pgid),
                start: facts.map(|f| StartToken(f.start_token)),
                same_user,
                name: process.name().to_string_lossy().into_owned(),
                exe: process.exe().map(std::path::Path::to_path_buf),
                cmdline: command_line(pid, process),
                cwd: process.cwd().map(std::path::Path::to_path_buf),
                alive,
            },
        );
    }
    let listeners = listeners
        .into_iter()
        .map(|mut l| {
            l.same_user = l
                .pid
                .and_then(|pid| processes.get(&pid))
                .and_then(|p: &ProcessInfo| p.same_user);
            l
        })
        .collect();
    Ok(Snapshot {
        listeners,
        processes,
        me: SelfInfo {
            pid: my_pid,
            pgid: my_facts.and_then(|f| f.pgid),
        },
        took: started.elapsed(),
    })
}

/// The process's arguments: on macOS the kernel's exact argv (empty for a process we may not read), elsewhere
/// `sysinfo`'s command line.
fn command_line(pid: u32, process: &sysinfo::Process) -> Vec<String> {
    if cfg!(target_os = "macos") {
        return vw_platform::process_argv(pid).unwrap_or_default();
    }
    process
        .cmd()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}
