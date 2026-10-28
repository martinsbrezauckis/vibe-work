//! Windows containment: one kill-on-close Job Object per shell. Closing the job handle kills every
//! process in it; the OS also closes it if the app is killed hard, so no shell outlives the app.
//!
//! Depends on: win32job (safe wrapper over the Job Object API), portable-pty (`Child::as_raw_handle`).
//! Called by: `terminal`.
//! Known limit: portable-pty cannot start the `ConPTY` child
//! suspended, so the shell is assigned right after `CreateProcessW` returns. A process the shell
//! starts in that window (before its own startup and profile have even run) is outside the job.
//! Closing the pseudo console still ends console-attached processes. portable-pty keeps its attribute
//! list private, so a fix needs our own `ConPTY` spawn with `PROC_THREAD_ATTRIBUTE_JOB_LIST`.
//! Proven on Windows 11 (`vw_terminal_windows.rs`): a `ping` the shell started is in the job
//! and gone by pid after close.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use portable_pty::Child;
use win32job::{ExtendedLimitInfo, Job};

/// How often [`Containment::terminate`] looks again while it waits (each look runs `tasklist`).
const POLL: Duration = Duration::from_millis(100);

/// The Job Object of one shell.
#[derive(Debug)]
pub(crate) struct Containment {
    job: Mutex<Option<Job>>,
}

impl Containment {
    /// Creates the job and assigns `child` (by its process handle) to it.
    pub(crate) fn assign(child: &dyn Child) -> Result<Self, String> {
        let mut limits = ExtendedLimitInfo::new();
        limits.limit_kill_on_job_close();
        let job = Job::create_with_limit_info(&limits).map_err(|e| e.to_string())?;
        let handle = child
            .as_raw_handle()
            .ok_or("the shell has no process handle")?;
        // The handle is owned by portable-pty's child for its whole life; the job only uses it
        // during this call. A pointer-sized handle value, not dereferenced here.
        job.assign_process(handle as isize)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            job: Mutex::new(Some(job)),
        })
    }

    /// No Unix session on Windows; a method to match the Unix side.
    #[allow(clippy::unused_self)]
    pub(crate) fn unix_session(&self) -> Option<u32> {
        None
    }

    /// Process ids in the job right now.
    pub(crate) fn job_members(&self) -> Vec<u32> {
        self.job
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|job| job.query_process_id_list().ok())
            .map(|ids| {
                ids.into_iter()
                    .filter_map(|id| u32::try_from(id).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Same as [`Containment::job_members`] (the name the Unix side uses).
    pub(crate) fn members(&self) -> Vec<u32> {
        self.job_members()
    }

    /// Closes the job (kills every member at once; Windows has no hang-up equivalent for console
    /// processes we could send to the whole tree) and waits up to `timeout` for the member list to
    /// empty. `grace` is unused on Windows. Returns true when nothing is left.
    pub(crate) fn terminate(&self, _grace: Duration, timeout: Duration) -> bool {
        let pids = self.job_members();
        self.kill_now();
        let deadline = Instant::now() + timeout;
        loop {
            let live = live_pids();
            if pids.iter().all(|pid| !live.contains(pid)) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(POLL);
        }
    }

    /// Closes the job handle: kill-on-close ends every process in it.
    pub(crate) fn kill_now(&self) {
        drop(
            self.job
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
    }
}

/// Process ids alive right now, from `tasklist` (CSV: `"name","pid",...`; no unsafe API needed).
/// An empty set when `tasklist` cannot run (then `terminate` reports success after the job close,
/// which is what kill-on-close guarantees).
fn live_pids() -> std::collections::HashSet<u32> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.split("\",\"").nth(1)?.parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}
