//! Precise process identity on macOS and Windows, and a pid-reuse-proof terminate on Windows.
//!
//! A pid alone is not an identity: the OS reuses pids. `vw-procmon` binds every process it may stop to
//! `(pid, start token)`. On Linux it reads the token from `/proc` itself (no `unsafe` needed); here are the
//! two platforms where only an OS call gives it:
//! - macOS: `proc_pidinfo(PROC_PIDTBSDINFO)` -> `pbi_start_tvsec * 1e6 + pbi_start_tvusec`, plus the owner
//!   uid, the process group and the zombie state from the same record.
//! - Windows: `GetProcessTimes` creation `FILETIME` (100 ns units). [`terminate_verified`] opens ONE handle,
//!   checks the token on that handle and terminates through it, so a pid reused between check and kill can
//!   never be hit (the handle pins the process object it was opened for).
//!
//! Also [`group_has_live_member`] (macOS): whether a process group still has a member that is not a zombie,
//! because macOS `killpg` answers EPERM for a group whose members all exited but are not reaped yet (block
//! Linux delivers to such a group and returns success).
//!
//! Depends on: libc (macOS), windows 0.62 (Windows). Called by: vw-procmon (`os::other`, `stop`, `signal`).
//! Invariant: vw-platform is the one crate allowed `unsafe` (with a documented SAFETY justification); every block
//! states its SAFETY argument. Other platforms get the safe fallbacks (`None` / `Unsupported`).
#![allow(unsafe_code)]

use std::time::Duration;

/// What the OS reports about a process that `sysinfo` does not give precisely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessFacts {
    /// Start token (macOS: microseconds since the epoch; Windows: creation FILETIME).
    pub start_token: u64,
    /// Owner uid (macOS only).
    pub uid: Option<u32>,
    /// Process group (macOS only).
    pub pgid: Option<u32>,
    /// True for an exited process still visible through an OS process record or handle.
    pub zombie: bool,
}

/// Why [`terminate_verified`] did not terminate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TerminateError {
    /// The process could not be opened (gone, or not ours to open).
    #[error("cannot open process {0}")]
    Open(u32),
    /// The process behind the pid started at another time: the pid was reused.
    #[error("process {pid} has another start time (pid reused)")]
    Reused {
        /// The pid.
        pid: u32,
    },
    /// The OS refused the terminate call.
    #[error("terminate of process {pid} failed: {reason}")]
    Failed {
        /// The pid.
        pid: u32,
        /// The OS error, with its Win32 code (`GetLastError`) as the OS reported it.
        reason: String,
    },
    /// Not available on this platform.
    #[error("not supported on this platform")]
    Unsupported,
}

/// Start token, owner and group of `pid`, or `None` when the process does not exist or may not be read.
#[must_use]
pub fn process_facts(pid: u32) -> Option<ProcessFacts> {
    imp::process_facts(pid)
}

/// macOS: `Some(true)` while the process group `pgid` has a member that is not a zombie, `Some(false)` when
/// every member it lists is a zombie or it lists none, `None` when the group cannot be listed. Other
/// platforms: `None`.
///
/// Why: `killpg` on macOS fails with EPERM for a group whose members have all exited but are not reaped yet
/// (verified on macOS 27 with a C probe: a group of one exited, unreaped `true` -> EPERM; after `waitpid` ->
/// ESRCH), where Linux returns success. A stop that signals the group right after its target died must not
/// report that as a failure; a group with a live member (another user's process, say) keeps the error.
#[must_use]
pub fn group_has_live_member(pgid: u32) -> Option<bool> {
    imp::group_has_live_member(pgid)
}

/// Windows: terminates `pid` only if its start token still equals `start_token`, through one handle (no
/// window for pid reuse), then waits up to `wait` for it to exit. Returns `Ok(true)` when it exited in time.
///
/// # Errors
/// [`TerminateError`] when the process cannot be opened, was reused, or the call failed; `Unsupported`
/// on other platforms (Unix stops by signal in vw-procmon).
pub fn terminate_verified(
    pid: u32,
    start_token: u64,
    wait: Duration,
) -> Result<bool, TerminateError> {
    imp::terminate_verified(pid, start_token, wait)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Duration, ProcessFacts, TerminateError};

    /// `SZOMB` in `<sys/proc.h>`.
    const SZOMB: u32 = 5;

    pub(super) fn process_facts(pid: u32) -> Option<ProcessFacts> {
        let pid = i32::try_from(pid).ok()?;
        let size = i32::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
        // SAFETY: proc_bsdinfo is a plain C struct of integers and byte arrays, so all-zero is a valid value.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        // SAFETY: the buffer is a live, writable proc_bsdinfo and `size` is exactly its size, as
        // proc_pidinfo(PROC_PIDTBSDINFO) requires; the call writes at most `size` bytes.
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast::<libc::c_void>(),
                size,
            )
        };
        if read != size {
            return None;
        }
        let start = info
            .pbi_start_tvsec
            .checked_mul(1_000_000)?
            .checked_add(info.pbi_start_tvusec)?;
        Some(ProcessFacts {
            start_token: start,
            uid: Some(info.pbi_uid),
            pgid: Some(info.pbi_pgid),
            zombie: info.pbi_status == SZOMB,
        })
    }

    pub(super) fn terminate_verified(
        _pid: u32,
        _start_token: u64,
        _wait: Duration,
    ) -> Result<bool, TerminateError> {
        Err(TerminateError::Unsupported)
    }

    pub(super) fn group_has_live_member(pgid: u32) -> Option<bool> {
        let group = libc::pid_t::try_from(pgid).ok()?;
        // libproc's proc_listpgrppids returns a NUMBER of pids (proc_listpids' byte count divided by the pid
        // size), not bytes like proc_listpids itself. SAFETY: a null buffer with size 0 only asks for the count.
        let needed = unsafe { libc::proc_listpgrppids(group, std::ptr::null_mut(), 0) };
        let needed = usize::try_from(needed).ok()?;
        // Room for members that join between the two calls.
        let mut pids: Vec<libc::pid_t> = vec![0; needed + 16];
        let bytes = i32::try_from(pids.len() * std::mem::size_of::<libc::pid_t>()).ok()?;
        // SAFETY: `pids` is a live, writable buffer of exactly `bytes` bytes; the call writes at most that many.
        let listed = unsafe {
            libc::proc_listpgrppids(group, pids.as_mut_ptr().cast::<libc::c_void>(), bytes)
        };
        let members = pids.get(..usize::try_from(listed).ok()?)?;
        Some(
            members
                .iter()
                .filter_map(|p| u32::try_from(*p).ok())
                .filter(|p| *p > 0)
                .any(|p| process_facts(p).is_some_and(|f| !f.zombie)),
        )
    }
}

#[cfg(all(test, windows))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod windows_tests {
    use super::process_facts;

    #[test]
    fn exited_process_with_a_retained_handle_is_not_live() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "exit /B 7"])
            .spawn()
            .unwrap();
        let pid = child.id();
        assert_eq!(child.wait().unwrap().code(), Some(7));
        // `child` still holds the process handle: opening that PID and reading its creation
        // FILETIME can succeed even though it has exited.
        let facts = process_facts(pid).expect("retained handle keeps exited process queryable");
        assert!(
            facts.zombie,
            "exited process must not retain its lease: {facts:?}"
        );
    }
}

#[cfg(windows)]
mod imp {
    use super::{Duration, ProcessFacts, TerminateError};
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
    };

    /// A process handle closed on drop.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful OpenProcess and is closed exactly once, here.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<Handle> {
        // SAFETY: OpenProcess takes plain values and returns an owned handle or an error.
        unsafe { OpenProcess(access, false, pid) }.ok().map(Handle)
    }

    fn creation(handle: &Handle) -> Option<u64> {
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: the handle is open with PROCESS_QUERY_LIMITED_INFORMATION and the four out-pointers are
        // valid, writable FILETIMEs on this stack frame.
        unsafe {
            GetProcessTimes(
                handle.0,
                &raw mut created,
                &raw mut exited,
                &raw mut kernel,
                &raw mut user,
            )
        }
        .ok()?;
        Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    /// Windows has no process groups to signal (stops go through `terminate_verified`).
    pub(super) fn group_has_live_member(_pgid: u32) -> Option<bool> {
        None
    }

    pub(super) fn process_facts(pid: u32) -> Option<ProcessFacts> {
        let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE)?;
        // OpenProcess and GetProcessTimes also succeed for an exited process while a caller
        // retains its handle. A zero-time wait distinguishes that object from a running process.
        // SAFETY: this is a valid handle opened with PROCESS_SYNCHRONIZE; no wait blocks.
        let zombie = match unsafe { WaitForSingleObject(handle.0, 0) } {
            WAIT_OBJECT_0 => true,
            WAIT_TIMEOUT => false,
            _ => return None,
        };
        Some(ProcessFacts {
            start_token: creation(&handle)?,
            uid: None,
            pgid: None,
            zombie,
        })
    }

    pub(super) fn terminate_verified(
        pid: u32,
        start_token: u64,
        wait: Duration,
    ) -> Result<bool, TerminateError> {
        let handle = open(
            pid,
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
        )
        .ok_or(TerminateError::Open(pid))?;
        if creation(&handle) != Some(start_token) {
            return Err(TerminateError::Reused { pid });
        }
        // SAFETY: the handle is open with PROCESS_TERMINATE and refers to the process whose start token was
        // just checked on this same handle.
        let terminated = unsafe { TerminateProcess(handle.0, 1) };
        let millis = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: the handle is open with PROCESS_SYNCHRONIZE.
        let waited = unsafe { WaitForSingleObject(handle.0, millis) };
        match terminated {
            Ok(()) => Ok(waited == WAIT_OBJECT_0),
            // TerminateProcess answers ERROR_ACCESS_DENIED for a process that is already exiting (Windows 11:
            // the `conhost.exe` of a console process whose client was terminated a moment
            // earlier). The handle is the verified process, so if it has exited by now the stop succeeded.
            Err(_) if waited == WAIT_OBJECT_0 => Ok(true),
            Err(e) => Err(TerminateError::Failed {
                pid,
                reason: format!("{} (HRESULT 0x{:08X})", e.message(), e.code().0),
            }),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::{Duration, ProcessFacts, TerminateError};

    pub(super) fn process_facts(_pid: u32) -> Option<ProcessFacts> {
        None
    }

    pub(super) fn group_has_live_member(_pgid: u32) -> Option<bool> {
        None
    }

    pub(super) fn terminate_verified(
        _pid: u32,
        _start_token: u64,
        _wait: Duration,
    ) -> Result<bool, TerminateError> {
        Err(TerminateError::Unsupported)
    }
}
