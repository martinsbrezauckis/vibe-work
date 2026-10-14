//! Unix containment: the shell is a session leader (portable-pty runs `setsid()` before exec,
//! `unix.rs:257` in 0.9.0), so its pid is the session id of every process it starts, including
//! background jobs, which interactive job control moves into their own process groups (so a
//! `killpg` of the shell's group alone would miss `sleep 1000 &`; ShellX `terminal.rs:728-761`
//! killed session members for the same reason).
//!
//! Depends on: nix (`getsid`, `kill`), `/proc` on Linux, `ps -axo pid=` on other Unix systems (macOS
//! has no `/proc`; this path is not exercised by the Linux tests, see the block report).
//! Called by: `contain` users (`terminal`).
//! Invariants:
//! - Members are found by session id at the moment of each pass, so processes started between the
//!   hang-up and the kill are caught by the second pass.
//! - A process that calls `setsid()` itself leaves the session and is not found: documented
//!   limit (no cgroups); the process monitor can still attribute it by cwd.
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::{Pid, getsid};
use tracing::debug;

/// How often [`Containment::terminate`] looks again while it waits.
const POLL: Duration = Duration::from_millis(20);

/// The session of one shell.
#[derive(Debug)]
pub(crate) struct Containment {
    leader: u32,
}

impl Containment {
    /// Containment for the shell with process id `pid` (portable-pty always reports one on Unix).
    pub(crate) fn new(pid: Option<u32>) -> Result<Self, String> {
        let leader = pid.ok_or("the shell has no process id")?;
        i32::try_from(leader).map_err(|_| format!("process id {leader} is out of range"))?;
        Ok(Self { leader })
    }

    /// The session id shared by the tree (the shell's pid). `Option` to match the Windows side.
    #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn unix_session(&self) -> Option<u32> {
        Some(self.leader)
    }

    /// Always empty on Unix (Windows lists its Job Object); a method to match the Windows side.
    #[allow(clippy::unused_self)]
    pub(crate) fn job_members(&self) -> Vec<u32> {
        Vec::new()
    }

    /// Every live process whose session id is the shell's pid (the shell included while alive).
    pub(crate) fn members(&self) -> Vec<u32> {
        let Ok(leader) = i32::try_from(self.leader) else {
            return Vec::new();
        };
        all_pids()
            .into_iter()
            .filter(|&pid| {
                i32::try_from(pid)
                    .ok()
                    .and_then(|raw| getsid(Some(Pid::from_raw(raw))).ok())
                    .is_some_and(|sid| sid.as_raw() == leader)
            })
            .collect()
    }

    fn signal_members(&self, signal: Signal) -> usize {
        let mut members = self.members();
        // Jobs first, the leader last, so the shell cannot start new jobs in between (ShellX order).
        members.sort_by_key(|pid| *pid == self.leader);
        for pid in &members {
            if let Ok(raw) = i32::try_from(*pid) {
                // ESRCH (already gone) is fine; EPERM cannot happen for our own children.
                let _ = kill(Pid::from_raw(raw), signal);
                if signal == Signal::SIGHUP {
                    // A stopped job only acts on the hang-up once it runs again.
                    let _ = kill(Pid::from_raw(raw), Signal::SIGCONT);
                }
            }
        }
        members.len()
    }

    /// Hangs up every member, waits up to `grace` for them to leave, then kills what is left and
    /// waits up to `timeout` more. Returns true when no member is left.
    pub(crate) fn terminate(&self, grace: Duration, timeout: Duration) -> bool {
        let hung_up = self.signal_members(Signal::SIGHUP);
        if hung_up == 0 {
            return true;
        }
        if wait_until(|| self.members().is_empty(), grace) {
            return true;
        }
        let killed = self.signal_members(Signal::SIGKILL);
        debug!(
            leader = self.leader,
            hung_up, killed, "terminal tree killed"
        );
        wait_until(|| self.members().is_empty(), timeout)
    }

    /// Kills every member at once without waiting (destructors).
    pub(crate) fn kill_now(&self) {
        self.signal_members(Signal::SIGKILL);
    }
}

fn wait_until(mut done: impl FnMut() -> bool, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// Every process id on the system.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn all_pids() -> Vec<u32> {
    std::fs::read_dir("/proc")
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Every process id on the system (macOS and other Unix without `/proc`).
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn all_pids() -> Vec<u32> {
    std::process::Command::new("/bin/ps")
        .args(["-axo", "pid="])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}
