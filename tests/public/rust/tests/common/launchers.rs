//! The real Vibe process behind a Windows launcher, and a process's descendants.
//!
//! Windows has no `exec`: started through `uv tool install`'s `vibe-app-server.exe`, Vibe runs as the interpreter
//! under the venv's `python.exe` launcher under uv's launcher, and all three name the server program. Vibe's session lease records the interpreter's pid, and suspending or killing only the launcher leaves
//! the interpreter running. On Unix a launcher replaces itself, so the started pid is Vibe.
//! Depends on: vw-procmon's process snapshot. Used by the real-process fault launcher and the tests' second-Vibe
//! lease helper.

/// The Vibe process behind `pid`, a server program that already answered (so its launchers have started everything):
/// from `pid` down, the only child that names the same program, as long as there is one. `pid` itself on Unix or when
/// nothing names it below.
#[must_use]
pub fn interpreter(pid: u32) -> u32 {
    if !cfg!(windows) {
        return pid;
    }
    let Ok(snapshot) = vw_procmon::snapshot() else {
        return pid;
    };
    let Some(program) = snapshot.alive(pid).and_then(|p| p.cmdline.first().cloned()) else {
        return pid;
    };
    let mut current = pid;
    // A launcher has exactly one child; more than one (or none) ends the chain.
    for _ in 0..8 {
        let children: Vec<u32> = snapshot
            .processes
            .values()
            .filter(|p| p.alive && p.ppid == Some(current) && snapshot.parent_of(p).is_some())
            .filter(|p| p.cmdline.iter().take(2).any(|arg| arg == &program))
            .map(|p| p.pid)
            .collect();
        let [child] = children.as_slice() else { break };
        current = *child;
    }
    current
}

/// `root` and every live process below it (start-consistent parent links).
#[must_use]
pub fn tree(root: u32) -> Vec<u32> {
    let Ok(snapshot) = vw_procmon::snapshot() else {
        return vec![root];
    };
    let mut found = vec![root];
    let mut index = 0;
    while index < found.len() {
        let parent = found[index];
        for process in snapshot.processes.values() {
            if process.alive
                && process.ppid == Some(parent)
                && snapshot.parent_of(process).is_some()
                && !found.contains(&process.pid)
            {
                found.push(process.pid);
            }
        }
        index += 1;
    }
    found
}

/// Stops (`stop`) or continues one process: `NtSuspendProcess` / `NtResumeProcess` (ntdll, stable since Windows XP),
/// called directly because PowerShell's `Add-Type` took one to three seconds per call, so a "suspend now" landed late.
/// True when the call succeeded (a process that is already gone counts as not done). Windows only.
#[cfg(windows)]
pub fn suspend(pid: u32, stop: bool) -> bool {
    use std::ffi::c_void;
    const PROCESS_SUSPEND_RESUME: u32 = 0x0800;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSuspendProcess(handle: *mut c_void) -> i32;
        fn NtResumeProcess(handle: *mut c_void) -> i32;
    }
    // SAFETY: plain Win32 calls on a handle this function opens and closes; no memory is shared.
    unsafe {
        let handle = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
        if handle.is_null() {
            return false;
        }
        let status = if stop {
            NtSuspendProcess(handle)
        } else {
            NtResumeProcess(handle)
        };
        CloseHandle(handle);
        status == 0
    }
}
