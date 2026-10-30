//! The ids of the current server leader and its crash watchdog, for the synchronous last-resort stop at quit.
//!
//! Depends on: nix (Unix signals). Called by: `child` (records and clears the ids), `supervisor` (`kill_now`).
//! Invariants:
//! - An id is recorded only while its process is this process's unreaped child, so a signal sent by id can never
//!   reach a reused id: the watchdog's id is cleared under the lock before it is reaped, the leader's right after
//!   tokio has reaped it (before the rest of its group is killed and reaped).
//! - A clear only removes the id it was given: a stale clear from a stopped server never removes the id of the
//!   next server start.
//! - `kill_now` never waits and never reaps: reaping stays with the owner task (or with the OS once this process
//!   has exited), so tokio's and std's own handles never wait on an id that something else already reaped.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Default)]
struct Ids {
    leader: Option<u32>,
    watchdog: Option<u32>,
}

/// Shared by the supervisor and the handles of its current server start.
#[derive(Clone, Default)]
pub(crate) struct LiveIds(Arc<Mutex<Ids>>);

impl LiveIds {
    fn locked(&self) -> MutexGuard<'_, Ids> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn set_leader(&self, pid: Option<u32>) {
        self.locked().leader = pid;
    }

    // Only the Unix crash watchdog exists; Windows has none to record.
    #[cfg(unix)]
    pub(crate) fn set_watchdog(&self, pid: Option<u32>) {
        self.locked().watchdog = pid;
    }

    /// The leader `pid` has been reaped.
    pub(crate) fn leader_gone(&self, pid: Option<u32>) {
        let mut ids = self.locked();
        if pid.is_some() && ids.leader == pid {
            ids.leader = None;
        }
    }

    /// Runs `reap` (kill and wait for the watchdog `pid`) with the id cleared first, under the lock, so a concurrent
    /// `kill_now` either signals the still unreaped watchdog or sees no id.
    pub(crate) fn watchdog_gone(&self, pid: u32, reap: impl FnOnce()) {
        let mut ids = self.locked();
        if ids.watchdog == Some(pid) {
            ids.watchdog = None;
        }
        reap();
    }

    /// SIGKILL to the leader's whole process group and to the watchdog, if they are still unreaped children.
    /// Returns whether a leader id was recorded. Errors mean the target is already gone: ignored.
    pub(crate) fn kill_now(&self) -> bool {
        let ids = self.locked();
        #[cfg(unix)]
        {
            use nix::sys::signal::{Signal, kill, killpg};
            use nix::unistd::Pid;
            let pid = |id: u32| i32::try_from(id).ok().map(Pid::from_raw);
            if let Some(leader) = ids.leader.and_then(pid) {
                let _ = killpg(leader, Signal::SIGKILL);
            }
            // The watchdog must not outlive the app either (it would only kill an already empty group).
            if let Some(watchdog) = ids.watchdog.and_then(pid) {
                let _ = kill(watchdog, Signal::SIGKILL);
            }
        }
        // Windows: the server's kill-on-close job object ends the whole tree when this process exits.
        ids.leader.is_some()
    }
}
