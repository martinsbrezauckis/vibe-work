//! Detached browser commands for the app's pairing and task launchers. Uses `LaunchServices` on macOS and native
//! discovery elsewhere; never reads a browser profile or puts the browser in an agent process job.
use super::{Browser, InstallationIssue};
use std::{
    path::Path,
    process::{Command, Stdio},
};

#[cfg(any(target_os = "macos", test))]
fn mac_command(browser: Browser, url: &str) -> Command {
    let mut command = Command::new("/usr/bin/open");
    command.args([
        "-g",
        "-b",
        match browser {
            Browser::Chrome => "com.google.Chrome",
            Browser::Edge => "com.microsoft.edgemac",
        },
    ]);
    if !url.is_empty() {
        command.arg(url);
    }
    command
}

/// Build a detached native browser launch. An empty URL starts the browser without selecting a page;
/// the authenticated extension creates each chat's own window after it reconnects.
///
/// # Errors
/// Returns the installation issue when native discovery cannot find the requested browser.
pub fn launch_command(
    browser: Browser,
    home: &Path,
    url: &str,
) -> Result<Command, InstallationIssue> {
    launch_command_with_profile(browser, home, url, None)
}

/// Build a detached launch for a known profile directory. macOS retains its `LaunchServices` launch.
///
/// # Errors
/// Returns the installation issue when native discovery cannot find the requested browser.
pub fn launch_command_with_profile(
    browser: Browser,
    home: &Path,
    url: &str,
    profile: Option<&str>,
) -> Result<Command, InstallationIssue> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let _ = (home, profile);
        mac_command(browser, url)
    };
    #[cfg(not(target_os = "macos"))]
    let mut command = native_command(&super::discover(browser, home)?, url, profile);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    Ok(command)
}

#[cfg(unix)]
#[allow(
    unsafe_code,
    reason = "setsid is an async-signal-safe OS call immediately before exec"
)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the child invokes only setsid and constructs an OS error; no allocation, locks or parent state.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
    };
    // A browser must outlive the app even when its launcher inherited a job. Refuse a prohibited breakaway
    // instead of silently attaching the person's browser to a kill-on-close job.
    command.creation_flags(
        (CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS).0,
    );
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn native_command(executable: &Path, url: &str, profile: Option<&str>) -> Command {
    let mut command = Command::new(executable);
    if let Some(profile) = profile {
        command.arg(format!("--profile-directory={profile}"));
    }
    if !url.is_empty() {
        command.arg(url);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mac_launch_uses_bundle_identity_without_profile_or_executable_paths() {
        for (browser, bundle) in [
            (Browser::Chrome, "com.google.Chrome"),
            (Browser::Edge, "com.microsoft.edgemac"),
        ] {
            for url in ["", "chrome-extension://example/ui.html#ticket"] {
                let command = mac_command(browser, url);
                assert_eq!(command.get_program(), "/usr/bin/open");
                let mut expected = vec!["-g", "-b", bundle];
                if !url.is_empty() {
                    expected.push(url);
                }
                assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
            }
        }
    }
}
