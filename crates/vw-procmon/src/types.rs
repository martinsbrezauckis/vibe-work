//! Plain data the OS readers produce and every other module consumes: a process start token, one process
//! row, one listening socket, and a whole snapshot.
//!
//! Depends on: serde. Called by: `os` (builds snapshots), `attribution`, `stop`, `ledger`, `monitor`, tests
//! (synthetic snapshots for the pure attribution tests).
//! Invariants:
//! - Anything the OS would not tell us is `None`, never a guess: an unreadable process has `same_user =
//!   None`, a listener whose socket owner we cannot map has `pid = None`.
//! - A [`StartToken`] is compared for equality (pid reuse) and for order (a parent never starts after its
//!   child); its unit differs per OS and is never shown as a time.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// When a process started, as the OS reports it, precise enough to tell two processes that reused one pid
/// apart: Linux `/proc/<pid>/stat` field 22 (clock ticks since boot), macOS `pbi_start_tvsec * 1e6 +
/// pbi_start_tvusec`, Windows creation `FILETIME` (100 ns units).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StartToken(pub u64);

/// A process identity that survives pid reuse: the pid plus its start token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRef {
    /// OS process id.
    pub pid: u32,
    /// Its start token at the time it was observed.
    pub start: StartToken,
}

/// One process as seen by a scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessInfo {
    /// OS process id.
    pub pid: u32,
    /// Parent pid, when readable.
    pub ppid: Option<u32>,
    /// Process group id (Unix only; `None` on Windows or when unreadable).
    pub pgid: Option<u32>,
    /// Start token, when readable.
    pub start: Option<StartToken>,
    /// `Some(true)` when the process runs as the same OS user as this app, `Some(false)` when another
    /// user's, `None` when the OS would not say.
    pub same_user: Option<bool>,
    /// Short process name (`comm` on Linux, image name elsewhere). The process can change it
    /// (`prctl(PR_SET_NAME)`, setproctitle): internal use only, never shown through a view.
    pub name: String,
    /// Executable path as the OS reports it (Linux `/proc/<pid>/exe`, `sysinfo` `exe()` elsewhere); `None`
    /// when unreadable (other users' processes). The only source of a process name in views.
    pub exe: Option<PathBuf>,
    /// Command line, when read (only for processes we may inspect; empty otherwise).
    pub cmdline: Vec<String>,
    /// Current working directory, when readable.
    pub cwd: Option<PathBuf>,
    /// False for a zombie (exited, not yet reaped): it holds no resources and counts as gone.
    pub alive: bool,
}

impl ProcessInfo {
    /// The basename of the OS-reported executable path, or `None` when it is unreadable. Never derived from
    /// the command line (argv[0] is chosen by the process: `exec -a`, setproctitle).
    #[must_use]
    pub fn exe_name(&self) -> Option<String> {
        self.exe
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    }

    /// The identity of this process, when its start token is known.
    #[must_use]
    pub fn reference(&self) -> Option<ProcessRef> {
        self.start.map(|start| ProcessRef {
            pid: self.pid,
            start,
        })
    }
}

/// One listening TCP socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketListener {
    /// Local address (`0.0.0.0` / `::` for a wildcard listener).
    pub address: IpAddr,
    /// Local port.
    pub port: u16,
    /// The process holding it, when it could be mapped (it may be another user's).
    pub pid: Option<u32>,
    /// Owner user check from the socket table itself (Linux keeps the uid of every socket), else `None`.
    pub same_user: Option<bool>,
}

/// This app's own place in the process table (never a stop target).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelfInfo {
    /// This process's pid.
    pub pid: u32,
    /// This process's group (Unix).
    pub pgid: Option<u32>,
}

/// Everything one scan saw.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Listening TCP sockets (IPv4 and IPv6).
    pub listeners: Vec<SocketListener>,
    /// Processes by pid.
    pub processes: BTreeMap<u32, ProcessInfo>,
    /// This app.
    pub me: SelfInfo,
    /// How long the scan took.
    pub took: Duration,
}

impl Snapshot {
    /// The process with `pid`, when the scan saw it alive.
    #[must_use]
    pub fn alive(&self, pid: u32) -> Option<&ProcessInfo> {
        self.processes.get(&pid).filter(|p| p.alive)
    }

    /// The parent of `child`, but only when it started no later than the child (a parent pid that was
    /// reused by a younger process is not the parent).
    #[must_use]
    pub fn parent_of(&self, child: &ProcessInfo) -> Option<&ProcessInfo> {
        let parent = self.processes.get(&child.ppid?)?;
        match (parent.start, child.start) {
            (Some(p), Some(c)) if p <= c => Some(parent),
            _ => None,
        }
    }

    /// True when `pid` is this app or one of its ancestors (walks the parent chain from this app).
    #[must_use]
    pub fn is_self_or_ancestor(&self, pid: u32) -> bool {
        let mut current = self.processes.get(&self.me.pid);
        if pid == self.me.pid {
            return true;
        }
        let mut steps = 0;
        while let Some(process) = current {
            if process.pid == pid {
                return true;
            }
            steps += 1;
            if steps > 256 {
                break;
            }
            current = process.ppid.and_then(|p| self.processes.get(&p));
        }
        false
    }

    /// True when `pid` descends from `root` (start-consistent parent chain, at most 256 steps).
    #[must_use]
    pub fn descends_from(&self, pid: u32, root: ProcessRef) -> bool {
        let mut current = self.processes.get(&pid);
        let mut steps = 0;
        while let Some(process) = current {
            if process.pid == root.pid {
                return process.start == Some(root.start);
            }
            steps += 1;
            if steps > 256 {
                return false;
            }
            current = self.parent_of(process);
        }
        false
    }

    /// Ports `pid` listens on.
    #[must_use]
    pub fn ports_of(&self, pid: u32) -> Vec<u16> {
        let mut ports: Vec<u16> = self
            .listeners
            .iter()
            .filter(|l| l.pid == Some(pid))
            .map(|l| l.port)
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }
}
