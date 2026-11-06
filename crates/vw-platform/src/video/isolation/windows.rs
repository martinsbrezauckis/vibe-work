//! Windows media worker launcher: a capability-free `AppContainer` in a bounded, kill-on-close Job.
//! The bootstrap parses no media. Only its three pipe handles reach the suspended child. Each run gets its own
//! `AppContainer` profile (`access::PREFIX`), deleted with its file grants when the run ends.
#![allow(
    clippy::wildcard_imports,
    reason = "Win32 process types and constants form this module's API surface"
)]
use std::{io, path::Path};
use windows::{
    Win32::{
        Foundation::*,
        Security::*,
        System::{Console::*, JobObjects::*, Threading::*},
    },
    core::PCWSTR,
};
mod access;
use access::{Owned, wide};
pub use access::{PREFIX, sweep};

use super::windows_limits::MEMORY;

fn cpu_time() -> i64 {
    super::windows_limits::cpu_time(
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
    )
}
const FLAGS: JOB_OBJECT_LIMIT = JOB_OBJECT_LIMIT(
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.0
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY.0
        | JOB_OBJECT_LIMIT_JOB_MEMORY.0
        | JOB_OBJECT_LIMIT_PROCESS_TIME.0
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS.0,
);

/// Launches the private copied helper under an `AppContainer` with access only to its input and executable.
/// # Errors
/// Any denied security setup or native launch; the child never runs before its limits are assigned.
pub fn launch(helper: &Path, input: &Path) -> io::Result<u32> {
    access::sweep();
    // Declared before the Job: it drops last, after the worker has exited and the Job is closed.
    let mut container = access::Container::create()?;
    container.grant(
        helper
            .parent()
            .ok_or_else(|| io::Error::other("helper directory"))?,
        true,
    )?;
    container.grant(helper, true)?;
    container.grant(input, false)?;
    // SAFETY: all native buffers and handles below outlive the synchronous launch and wait.
    unsafe {
        let job = Owned(CreateJobObjectW(None, PCWSTR::null()).map_err(io::Error::other)?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = FLAGS;
        limits.BasicLimitInformation.PerProcessUserTimeLimit = cpu_time();
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.ProcessMemoryLimit = MEMORY;
        limits.JobMemoryLimit = MEMORY;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            u32::try_from(size_of_val(&limits)).map_err(io::Error::other)?,
        )
        .map_err(io::Error::other)?;
        let process = spawn(helper, input, container.sid(), &job)?;
        if WaitForSingleObject(process.0, INFINITE) != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut exit = 1;
        GetExitCodeProcess(process.0, &raw mut exit).map_err(io::Error::other)?;
        Ok(exit)
    }
}

fn spawn(helper: &Path, input: &Path, sid: PSID, job: &Owned) -> io::Result<Owned> {
    let handles = [
        access::pipe(STD_INPUT_HANDLE)?,
        access::pipe(STD_OUTPUT_HANDLE)?,
        access::pipe(STD_ERROR_HANDLE)?,
    ];
    let inherited = handles.each_ref().map(|h| h.0);
    let caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        ..Default::default()
    };
    let child_policy: u32 =
        windows::Win32::System::WindowsProgramming::PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
    let mut attributes = access::Attributes::new(3)?;
    attributes.add(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize, &caps)?;
    attributes.add(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &inherited)?;
    attributes.add(
        PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY as usize,
        &child_policy,
    )?;
    let startup = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: u32::try_from(size_of::<STARTUPINFOEXW>()).map_err(io::Error::other)?,
            dwFlags: STARTF_USESTDHANDLES,
            hStdInput: inherited[0],
            hStdOutput: inherited[1],
            hStdError: inherited[2],
            ..Default::default()
        },
        lpAttributeList: attributes.pointer(),
    };
    // Paths originate in the parent's private staging directory. Quote them explicitly, rejecting embedded quotes.
    let helper_text = helper
        .to_str()
        .filter(|s| !s.contains(['"', '\0']))
        .ok_or_else(|| io::Error::other("helper path encoding"))?;
    let input_text = input
        .to_str()
        .filter(|s| !s.contains(['"', '\0']))
        .ok_or_else(|| io::Error::other("snapshot path encoding"))?;
    let name = wide(helper.as_os_str());
    let mut command = wide(std::ffi::OsStr::new(&format!(
        "\"{helper_text}\" --worker \"{input_text}\""
    )));
    let mut started = PROCESS_INFORMATION::default();
    // SAFETY: attribute buffers, strings and inheritable pipes are live until CreateProcessW returns.
    unsafe {
        CreateProcessW(
            PCWSTR(name.as_ptr()),
            Some(windows::core::PWSTR(command.as_mut_ptr())),
            None,
            None,
            true,
            // DETACHED_PROCESS: an AppContainer process cannot start a console of its own (its DLL initialization
            // fails with 0xC0000142); the worker only uses the inherited pipes.
            // The inherited environment is the bootstrap's SystemRoot and LOCALAPPDATA: Windows needs LOCALAPPDATA to
            // point the container's folders at its profile, so the worker cannot get an empty block.
            CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT | DETACHED_PROCESS,
            None,
            PCWSTR::null(),
            &raw const startup.StartupInfo,
            &raw mut started,
        )
        .map_err(io::Error::other)?;
        let process = Owned(started.hProcess);
        let thread = Owned(started.hThread);
        if let Err(error) = AssignProcessToJobObject(job.0, process.0) {
            let _ = TerminateProcess(process.0, 1);
            let _ = WaitForSingleObject(process.0, INFINITE);
            return Err(io::Error::other(error));
        }
        if ResumeThread(thread.0) == u32::MAX {
            return Err(io::Error::last_os_error()); // closing the job kills the suspended process
        }
        Ok(process)
    }
}

/// Refuses direct worker invocation unless Windows confirms both `AppContainer` and hard Job limits.
/// # Errors
/// An unrestricted token, absent limits or failed OS query. Called before stdin or media is parsed.
pub fn verify() -> Result<(), String> {
    verify_native().map_err(|_| "backend_unavailable: Windows media sandbox is not active".into())
}
fn verify_native() -> io::Result<()> {
    // SAFETY: GetTokenInformation and QueryInformationJobObject write to their correctly sized stack structures.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token)
            .map_err(io::Error::other)?;
        let token = Owned(token);
        let (mut container, mut returned) = (0u32, 0);
        GetTokenInformation(
            token.0,
            TokenIsAppContainer,
            Some((&raw mut container).cast()),
            4,
            &raw mut returned,
        )
        .map_err(io::Error::other)?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        QueryInformationJobObject(
            None,
            JobObjectExtendedLimitInformation,
            (&raw mut limits).cast(),
            u32::try_from(size_of_val(&limits)).map_err(io::Error::other)?,
            None,
        )
        .map_err(io::Error::other)?;
        if container != 1
            || limits.BasicLimitInformation.LimitFlags.0 & FLAGS.0 != FLAGS.0
            || limits.BasicLimitInformation.ActiveProcessLimit != 1
            || !(1..=MEMORY).contains(&limits.ProcessMemoryLimit)
            || !(1..=MEMORY).contains(&limits.JobMemoryLimit)
            || !(1..=cpu_time()).contains(&limits.BasicLimitInformation.PerProcessUserTimeLimit)
        {
            return Err(io::Error::other("unrestricted worker"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinary_process_cannot_claim_to_be_a_worker() {
        assert!(super::verify().is_err());
    }
    #[test]
    fn every_run_has_its_own_profile_and_none_outlives_it() {
        let first = super::access::Container::create().unwrap();
        let second = super::access::Container::create().unwrap();
        // SAFETY: both SIDs stay owned and valid for the comparison.
        assert!(unsafe { windows::Win32::Security::EqualSid(first.sid(), second.sid()) }.is_err());
        let names = [first.name().to_owned(), second.name().to_owned()];
        let listed = super::access::profiles();
        assert!(names.iter().all(|name| listed.contains(name)), "{listed:?}");
        drop((first, second));
        let listed = super::access::profiles();
        assert!(
            names.iter().all(|name| !listed.contains(name)),
            "{listed:?}"
        );
    }
    #[test]
    fn sweep_removes_only_profiles_of_runs_that_ended() {
        let live = super::access::Container::create().unwrap();
        // A dead run: PID 0 is never a bootstrap. Registered directly, as a killed run would leave it.
        let dead = format!("{}00000000.0000000000000001.00000000", super::PREFIX);
        let name = super::wide(std::ffi::OsStr::new(&dead));
        // SAFETY: the name is NUL terminated; the returned SID is freed at once.
        let sid = unsafe {
            windows::Win32::Security::Isolation::CreateAppContainerProfile(
                windows::core::PCWSTR(name.as_ptr()),
                windows::core::PCWSTR(name.as_ptr()),
                windows::core::PCWSTR(name.as_ptr()),
                None,
            )
        }
        .unwrap();
        drop(super::access::Sid(sid));
        super::sweep();
        let listed = super::access::profiles();
        assert!(!listed.contains(&dead), "{listed:?}");
        assert!(listed.contains(&live.name().to_owned()), "{listed:?}");
    }
}
