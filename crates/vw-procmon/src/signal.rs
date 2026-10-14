//! Sending the stop: Unix TERM to the target (a pidfd on Linux) and its process group, wait, re-verify,
//! KILL; Windows a handle-pinned, start-token-checked `TerminateProcess` per tree member. Split out of `stop`
//! (which decides WHAT may be stopped); this module only carries out a `stop::Plan`.
//!
//! Depends on: `os` (liveness, identity), nix (Unix signals), rustix (Linux pidfd), vw-platform (Windows).
//! Called by: `stop` only.

#[cfg(unix)]
mod imp {
    //! TERM the target (pidfd on Linux) and its group, wait, re-verify, KILL, wait.
    use std::thread::sleep;
    use std::time::Instant;

    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    use crate::os;
    use crate::stop::{Plan, StopConfig, StopError};
    use crate::types::ProcessRef;

    fn send(group: u32, signal: Signal) -> Result<(), StopError> {
        let raw = i32::try_from(group).map_err(|_| StopError::Signal {
            pid: group,
            reason: "group id out of range".to_owned(),
        })?;
        match killpg(Pid::from_raw(raw), signal) {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
            // macOS answers EPERM for a group whose members have all exited but are not reaped yet (the
            // target died from the TERM a moment ago and its parent has not waited yet); Linux delivers and
            // succeeds. Such a group is stopped. A live member (another user's process) keeps the error.
            #[cfg(target_os = "macos")]
            Err(nix::errno::Errno::EPERM)
                if vw_platform::group_has_live_member(group) == Some(false) =>
            {
                Ok(())
            }
            Err(e) => Err(StopError::Signal {
                pid: group,
                reason: e.to_string(),
            }),
        }
    }

    fn wait(members: &[ProcessRef], grace: std::time::Duration, config: &StopConfig) -> Vec<u32> {
        let deadline = Instant::now() + grace;
        loop {
            let alive: Vec<u32> = members
                .iter()
                .filter(|m| os::is_alive(**m))
                .map(|m| m.pid)
                .collect();
            if alive.is_empty() || Instant::now() >= deadline {
                return alive;
            }
            sleep(config.poll);
        }
    }

    /// The target, pinned on Linux by a pidfd that was opened after the snapshot and whose process was
    /// re-verified by start token; elsewhere (or on kernels without pidfd) signalled by pid.
    struct Pinned {
        target: ProcessRef,
        #[cfg(target_os = "linux")]
        fd: Option<std::os::fd::OwnedFd>,
    }

    impl Pinned {
        /// `Ok(None)` when the target is gone; a refusal when its pid now belongs to another process.
        fn open(target: ProcessRef) -> Result<Option<Self>, StopError> {
            #[cfg(target_os = "linux")]
            let fd = {
                use rustix::process::{PidfdFlags, pidfd_open};
                let pid = i32::try_from(target.pid)
                    .ok()
                    .and_then(rustix::process::Pid::from_raw);
                match pid.map(|p| pidfd_open(p, PidfdFlags::empty())) {
                    Some(Ok(fd)) => Some(fd),
                    Some(Err(rustix::io::Errno::SRCH)) => return Ok(None),
                    // Kernel without pidfd (before 5.3) or another error: fall back to the pid.
                    _ => None,
                }
            };
            // Re-verify AFTER opening: if the start token still matches, the pidfd (if any) refers to the
            // verified process, because a pid cannot be reused while its process lives.
            if !os::is_alive(target) {
                return match os::identity(target.pid) {
                    Some(_) => Err(StopError::Refused(crate::stop::StopRefusal::PidReused {
                        pid: target.pid,
                    })),
                    None => Ok(None),
                };
            }
            Ok(Some(Self {
                target,
                #[cfg(target_os = "linux")]
                fd,
            }))
        }

        fn signal(&self, signal: Signal) -> Result<(), StopError> {
            let failed = |reason: String| StopError::Signal {
                pid: self.target.pid,
                reason,
            };
            #[cfg(target_os = "linux")]
            if let Some(fd) = &self.fd {
                let sig = if signal == Signal::SIGKILL {
                    rustix::process::Signal::KILL
                } else {
                    rustix::process::Signal::TERM
                };
                return match rustix::process::pidfd_send_signal(fd, sig) {
                    Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
                    Err(e) => Err(failed(e.to_string())),
                };
            }
            let raw = i32::try_from(self.target.pid)
                .map_err(|_| failed("pid out of range".to_owned()))?;
            match nix::sys::signal::kill(Pid::from_raw(raw), signal) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
                Err(e) => Err(failed(e.to_string())),
            }
        }
    }

    /// Returns (escalated, remaining pids).
    pub(crate) fn run(plan: &Plan, config: &StopConfig) -> Result<(bool, Vec<u32>), StopError> {
        let Some(pinned) = Pinned::open(plan.target)? else {
            return Ok((false, Vec::new()));
        };
        pinned.signal(Signal::SIGTERM)?;
        if let Some(group) = plan.group {
            send(group, Signal::SIGTERM)?;
        }
        let remaining = wait(&plan.members, config.term_grace, config);
        if remaining.is_empty() {
            return Ok((false, remaining));
        }
        // Re-verify before KILL: only members that are still the same identities are waited for; while one
        // of them lives, the group id cannot have been reused.
        if !plan.members.iter().any(|m| os::is_alive(*m)) {
            return Ok((false, Vec::new()));
        }
        pinned.signal(Signal::SIGKILL)?;
        if let Some(group) = plan.group {
            send(group, Signal::SIGKILL)?;
        }
        Ok((true, wait(&plan.members, config.kill_grace, config)))
    }
}

#[cfg(windows)]
mod imp {
    //! Terminate each member through a handle on which its start token was checked, target first (so it
    //! cannot respawn children while they are terminated).
    use crate::os;
    use crate::stop::{Plan, StopConfig, StopError};
    use vw_platform::{TerminateError, terminate_verified};

    /// Returns (escalated = false: Windows has no TERM phase, remaining pids).
    pub(crate) fn run(plan: &Plan, config: &StopConfig) -> Result<(bool, Vec<u32>), StopError> {
        debug_assert_eq!(plan.members.first(), Some(&plan.target));
        let mut remaining = Vec::new();
        for member in &plan.members {
            match terminate_verified(member.pid, member.start.0, config.kill_grace) {
                Ok(true) => {}
                Ok(false) => remaining.push(member.pid),
                // Gone already (cannot open / another process now) unless it still runs as itself.
                Err(TerminateError::Open(_) | TerminateError::Reused { .. }) => {
                    if os::is_alive(*member) {
                        remaining.push(member.pid);
                    }
                }
                Err(e) => {
                    let name = plan.names.get(&member.pid).map_or("?", String::as_str);
                    let role = if *member == plan.target {
                        "target"
                    } else {
                        "tree member"
                    };
                    return Err(StopError::Signal {
                        pid: member.pid,
                        reason: format!("{e} ({role} {name})"),
                    });
                }
            }
        }
        Ok((false, remaining))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use crate::stop::{Plan, StopConfig, StopError};

    pub(crate) fn run(plan: &Plan, _config: &StopConfig) -> Result<(bool, Vec<u32>), StopError> {
        Err(StopError::Signal {
            pid: plan.target.pid,
            reason: "unsupported platform".to_owned(),
        })
    }
}

pub(crate) use imp::run;
