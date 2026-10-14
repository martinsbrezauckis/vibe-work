//! One OS scan per platform behind one interface: [`snapshot`] (listening sockets + process table),
//! [`is_alive`] (cheap re-check of one identity while a stop waits) and [`fill_cmdline`].
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
