//! One OS scan per platform behind one interface: [`snapshot`] (listening sockets + process table),
//! [`is_alive`] (cheap re-check of one identity while a stop waits), [`port_held_by`] (cheap re-check of who
//! listens on one port) and [`fill_cmdline`].
//!
//! Depends on: `os::linux` (std `/proc` reader) or `os::other` (Windows/macOS: listeners + sysinfo +
//! vw-platform). Called by: `monitor`, `stop`, `ledger`. Other platforms get an `Unsupported` scan error.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(windows, target_os = "macos"))]
mod other;

use crate::error::ScanError;
use crate::types::{ProcessRef, Snapshot};

/// Scans the listening sockets and the process table.
///
/// # Errors
/// [`ScanError`] only when the OS tables themselves cannot be read (single unreadable processes are not
/// errors), or on an unsupported platform.
pub fn snapshot() -> Result<Snapshot, ScanError> {
    #[cfg(target_os = "linux")]
    return linux::snapshot();
    #[cfg(any(windows, target_os = "macos"))]
    return other::snapshot();
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    return Err(ScanError::Unsupported);
}

/// True when the process `process` names still runs (same start token, not a zombie).
#[must_use]
pub fn is_alive(process: ProcessRef) -> bool {
    #[cfg(target_os = "linux")]
    return linux::is_alive(process);
    #[cfg(any(windows, target_os = "macos"))]
    return other::is_alive(process);
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    return {
        let _ = process;
        false
    };
}

/// True while `port` has at least one listening TCP socket and every one of them is held by one of `processes`,
/// each of which still runs (same start token). A re-check of one port against known processes, no process-table
/// scan: a process that closed its socket, or another program's listener on the port, fails it. Fails closed when
/// the socket table cannot be read. macOS cannot see other users' listeners (as in [`snapshot`]).
#[must_use]
pub fn port_held_by(port: u16, processes: &[ProcessRef]) -> bool {
    #[cfg(target_os = "linux")]
    return linux::port_held_by(port, processes);
    #[cfg(any(windows, target_os = "macos"))]
    return other::port_held_by(port, processes);
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    return {
        let _ = (port, processes);
        false
    };
}

/// Reads the command line of the given pids into the snapshot where it is still empty (Linux reads command
/// lines lazily; elsewhere the scan already has them).
pub fn fill_cmdline(snapshot: &mut Snapshot, pids: &[u32]) {
    #[cfg(target_os = "linux")]
    for pid in pids {
        if let Some(process) = snapshot.processes.get_mut(pid)
            && process.cmdline.is_empty()
        {
            process.cmdline = linux::read_cmdline(*pid);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (snapshot, pids);
}

/// The command line of `pid` (Linux reads it now; elsewhere the scan already has it, so this is empty).
#[must_use]
pub fn cmdline(pid: u32) -> Vec<String> {
    #[cfg(target_os = "linux")]
    return linux::read_cmdline(pid);
    #[cfg(not(target_os = "linux"))]
    return {
        let _ = pid;
        Vec::new()
    };
}

/// The identity (pid + start token) of `pid` right now, without a full scan.
#[must_use]
pub fn identity(pid: u32) -> Option<ProcessRef> {
    #[cfg(target_os = "linux")]
    return linux::identity(pid);
    #[cfg(any(windows, target_os = "macos"))]
    return vw_platform::process_facts(pid)
        .filter(|f| !f.zombie)
        .map(|f| ProcessRef {
            pid,
            start: crate::types::StartToken(f.start_token),
        });
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    return {
        let _ = pid;
        None
    };
}

#[cfg(test)]
mod tests {
    use crate::types::{ProcessRef, StartToken};

    /// PREDICT: the macOS scan never read the POSIX session (`sid: None`), so a process a chat's terminal
    /// started detached (`(sleep N &)`: its parent is init) had only its folder as evidence, and the kill guard
    /// named every chat of that folder. macOS reports any same-user process's session like Linux; the scan
    /// now carries it, so the registered terminal shell that leads that session owns the process as a fact.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_detached_process_keeps_the_session_of_the_shell_that_started_it() {
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", "(sleep 30 >/dev/null 2>&1 & echo $!)"])
            .output()
            .unwrap();
        let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        let ours = u32::try_from(nix::unistd::getsid(None).unwrap().as_raw()).unwrap();
        let snapshot = super::snapshot().unwrap();
        let detached = snapshot.processes.get(&pid).cloned();
        let me = snapshot.processes.get(&std::process::id()).cloned();
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(pid).unwrap()),
            nix::sys::signal::Signal::SIGKILL,
        );
        let detached = detached.expect("the detached sleep is in the scan");
        assert_ne!(
            detached.ppid,
            Some(std::process::id()),
            "it left its parent"
        );
        assert_eq!(detached.sid, Some(ours), "the detached process's session");
        assert_eq!(me.and_then(|p| p.sid), Some(ours), "this process's session");
    }

    /// A port counts as held only by the processes holding every listener on it, each still the same process.
    #[cfg(any(target_os = "linux", windows, target_os = "macos"))]
    #[test]
    fn a_port_is_held_only_by_its_current_listeners_process() {
        if std::env::var_os("VW_PROCMON_ISOLATED_PORT_TEST").is_none() {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "os::tests::a_port_is_held_only_by_its_current_listeners_process",
                    "--nocapture",
                ])
                .env("VW_PROCMON_ISOLATED_PORT_TEST", "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&child.stdout);
            assert!(
                child.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed"),
                "isolated listener check failed: {} {}",
                stdout,
                String::from_utf8_lossy(&child.stderr)
            );
            return;
        }
        let me = super::identity(std::process::id()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(super::port_held_by(port, &[me]));
        assert!(!super::port_held_by(port, &[]), "no known process");
        let reused = ProcessRef {
            pid: me.pid,
            start: StartToken(me.start.0.wrapping_add(1)),
        };
        assert!(
            !super::port_held_by(port, &[reused]),
            "another process with the same pid"
        );
        #[cfg(unix)]
        if let Some(parent) = super::identity(std::os::unix::process::parent_id()) {
            assert!(
                !super::port_held_by(port, &[parent]),
                "a process that does not hold the listener"
            );
        }
        let kept = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        drop(listener);
        assert!(
            !super::port_held_by(port, &[me]),
            "the socket was closed (the process still holds another one)"
        );
        drop(kept);
    }
}
