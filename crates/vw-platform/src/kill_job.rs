//! Windows: runs a program in a job of its own that ends together with this process, so a stop of this process ends
//! every program the started one left behind.
//!
//! Why: Vibe stops a Git Bash command with `taskkill /F /T`, which follows the parent links of the process tree. When
//! bash forks (`sleep 90 && echo done`, `cd app && npm test`), MSYS starts the program as a new Windows process whose
//! parent has already exited, so the tree walk never reaches it and it runs on after Stop or the bash timeout. A job
//! with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` holds every process started inside it, whatever its parent; the only
//! handle to the job is this process's, so when this process ends (taskkill, or its own exit after the program
//! finished) Windows closes the handle and ends everything still in the job, as Vibe's process-group kill does on Linux
//! and macOS.
//!
//! The command line is passed on as written: only the program name is replaced by `program`, the rest of this
//! process's own command line (quotes and all) stays byte for byte, so the program parses exactly what Vibe wrote.
//!
//! Depends on: the `windows` crate (job objects, process creation). Called by: `vw-agent-tools` (its `bash.exe` mode).

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::Environment::GetCommandLineW;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, GetExitCodeProcess, INFINITE, PROCESS_INFORMATION,
    ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

/// The part of a Windows command line after the program name, by the rules the C runtime uses for `argv[0]`: a
/// quoted name runs to the next quote (no escapes in the name), an unquoted one to the first space or tab. Pure.
#[must_use]
pub fn after_program_name(line: &[u16]) -> &[u16] {
    const QUOTE: u16 = b'"' as u16;
    const SPACE: u16 = b' ' as u16;
    const TAB: u16 = b'\t' as u16;
    let end = if line.first() == Some(&QUOTE) {
        line[1..]
            .iter()
            .position(|&c| c == QUOTE)
            .map_or(line.len(), |i| i + 2)
    } else {
        line.iter()
            .position(|&c| c == SPACE || c == TAB)
            .unwrap_or(line.len())
    };
    &line[end..]
}

/// `program` quoted as a command line's program name, then `rest` as it was. Pure.
#[must_use]
pub fn command_line(program: &Path, rest: &[u16]) -> Vec<u16> {
    let mut line: Vec<u16> = Vec::with_capacity(rest.len() + 260);
    line.push(u16::from(b'"'));
    line.extend(program.as_os_str().encode_wide());
    line.push(u16::from(b'"'));
    line.extend_from_slice(rest);
    line.push(0);
    line
}

/// Closes a handle when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful Win32 call in this module and is closed exactly once here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Runs `program` with the rest of this process's command line in a new kill-on-close job, with this process's
/// standard handles, waits for it and returns its exit code. Everything the program started that is still running
/// ends when it does (the job closes before this returns), and also when this process is ended while it waits.
///
/// # Errors
/// The job or the process could not be created, or the process could not be put in the job (it is ended then and
/// never runs).
pub fn run_in_kill_job(program: &Path) -> io::Result<u32> {
    // SAFETY: GetCommandLineW returns this process's command line, valid and NUL-terminated for its whole life.
    let own = unsafe { GetCommandLineW() };
    // SAFETY: `own` is a valid NUL-terminated wide string (above).
    let own = unsafe { own.as_wide() };
    let mut line = command_line(program, after_program_name(own));
    let name: Vec<u16> = program
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: no attributes, no name: a new unnamed job only this process holds a handle to.
    let job = Owned(unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(io::Error::other)?);
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` is a valid JOBOBJECT_EXTENDED_LIMIT_INFORMATION and the size passed is its own.
    unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .map_err(io::Error::other)?,
        )
    }
    .map_err(io::Error::other)?;

    let mut startup = STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).map_err(io::Error::other)?,
        dwFlags: STARTF_USESTDHANDLES,
        ..Default::default()
    };
    // SAFETY: GetStdHandle only reads this process's standard handles; a missing one is passed on as it is.
    unsafe {
        startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE).unwrap_or_default();
        startup.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE).unwrap_or_default();
        startup.hStdError = GetStdHandle(STD_ERROR_HANDLE).unwrap_or_default();
    }
    let mut started = PROCESS_INFORMATION::default();
    // Suspended until it is in the job, so nothing it starts can be outside it. Handles are inherited (the standard
    // handles Vibe gave this process); the environment and the working folder are this process's.
    // SAFETY: `name` and `line` are NUL-terminated wide strings that outlive the call (`line` is mutable, as the call
    // requires); `startup` and `started` are valid for writes.
    unsafe {
        CreateProcessW(
            PCWSTR(name.as_ptr()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED,
            None,
            PCWSTR::null(),
            &raw const startup,
            &raw mut started,
        )
    }
    .map_err(io::Error::other)?;
    let process = Owned(started.hProcess);
    let thread = Owned(started.hThread);
    // SAFETY: both handles are the live job and the suspended process created above.
    if let Err(error) = unsafe { AssignProcessToJobObject(job.0, process.0) } {
        // SAFETY: the process is ours and still suspended: it never ran.
        let _ = unsafe { TerminateProcess(process.0, 1) };
        return Err(io::Error::other(error));
    }
    // SAFETY: the main thread of the process created above, suspended once by CREATE_SUSPENDED.
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        // SAFETY: as above; the job also ends it when this process exits.
        let _ = unsafe { TerminateProcess(process.0, 1) };
        return Err(io::Error::last_os_error());
    }
    drop(thread);
    // SAFETY: a live process handle; INFINITE waits until it ends.
    if unsafe { WaitForSingleObject(process.0, INFINITE) } != WAIT_OBJECT_0 {
        return Err(io::Error::last_os_error());
    }
    let mut code = 0u32;
    // SAFETY: the process has ended; `code` is valid for the write.
    unsafe { GetExitCodeProcess(process.0, &raw mut code) }.map_err(io::Error::other)?;
    // Dropping the job here closes its only handle: whatever the program left running ends now, before the caller
    // exits with the program's own code.
    drop(job);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn the_rest_of_the_command_line_is_kept_as_written() {
        assert_eq!(
            after_program_name(&wide(r#""C:\a b\bash.exe" -c "echo \"x\" && sleep 1""#)),
            wide(r#" -c "echo \"x\" && sleep 1""#)
        );
        assert_eq!(
            after_program_name(&wide(r"C:\shell\bash.exe -c 'a b'")),
            wide(r" -c 'a b'")
        );
        assert_eq!(after_program_name(&wide("bash.exe")), wide(""));
        assert_eq!(after_program_name(&wide("\"unterminated")), wide(""));
        assert_eq!(
            command_line(
                Path::new(r"C:\Program Files\Git\bin\bash.exe"),
                &wide(" -c ls")
            ),
            [
                wide(r#""C:\Program Files\Git\bin\bash.exe" -c ls"#),
                vec![0]
            ]
            .concat()
        );
    }
}
