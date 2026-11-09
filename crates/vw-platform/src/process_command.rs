//! Commands started by the GUI have no Windows console window. Std, Tokio and job-wrapped builders share flags.
//! Depends on process-wrap/Tokio; called by app services and probes. Extra job/detachment flags remain additive.
use process_wrap::tokio::CommandWrap;
use std::{ffi::OsStr, process::Command};

/// A standard command that cannot flash a console when started by the desktop app.
pub fn hidden(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    set_creation_flags(&mut command, 0);
    command
}

/// The Tokio equivalent of [`hidden`].
pub fn hidden_tokio(program: impl AsRef<OsStr>) -> tokio::process::Command {
    hidden(program).into()
}

/// A hidden command whose flags survive process-wrap's suspended job assignment.
pub fn hidden_wrap(
    program: impl AsRef<OsStr>,
    init: impl FnOnce(&mut tokio::process::Command),
) -> CommandWrap {
    let mut command = hidden_tokio(program);
    init(&mut command);
    let command = CommandWrap::from(command);
    #[cfg(windows)]
    let command = {
        let mut command = command;
        command.wrap(process_wrap::tokio::CreationFlags(
            windows::Win32::System::Threading::PROCESS_CREATION_FLAGS(hidden_flags(0)),
        ));
        command
    };
    command
}

/// Apply extra Windows creation flags while retaining the no-console rule. Other systems ignore the flags.
pub fn set_creation_flags(command: &mut Command, extra_flags: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(hidden_flags(extra_flags));
    }
    #[cfg(not(windows))]
    let _ = (command, extra_flags);
}

#[cfg(windows)]
fn hidden_flags(extra_flags: u32) -> u32 {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    extra_flags | CREATE_NO_WINDOW
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn extra_flags_keep_no_window_and_job_wrappers_use_the_same_flags() {
        assert_eq!(hidden_flags(0x0000_0200 | 0x0100_0000), 0x0900_0200);
        let command = hidden_wrap("cmd", |_| {});
        let flags = command
            .get_wrap::<process_wrap::tokio::CreationFlags>()
            .unwrap();
        assert_eq!(flags.0.0, hidden_flags(0));
    }
}
