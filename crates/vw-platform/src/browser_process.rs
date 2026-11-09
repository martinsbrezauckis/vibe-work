//! Browser observation dispatch. Windows uses native process/window APIs; POSIX workers bound and reap pgrep.
use super::{Browser, ProcessState};
#[cfg(unix)]
use std::io::Read;
#[cfg(not(windows))]
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(windows)]
pub(super) fn inspect(browser: Browser) -> (ProcessState, Option<Vec<u32>>) {
    (super::windows::inspect(browser), None)
}

#[cfg(not(windows))]
pub(super) fn inspect(browser: Browser) -> (ProcessState, Option<Vec<u32>>) {
    let edge = browser == Browser::Edge;
    let mut command = {
        let mut command = Command::new(if cfg!(target_os = "macos") {
            "/usr/bin/pgrep"
        } else {
            "pgrep"
        });
        command.args([
            "-x",
            if cfg!(target_os = "macos") {
                if edge {
                    "Microsoft Edge"
                } else {
                    "Google Chrome"
                }
            } else if edge {
                "msedge"
            } else {
                "chrome"
            },
        ]);
        command
    };
    bounded(&mut command, Duration::from_secs(2))
}

#[cfg(not(windows))]
pub(super) fn bounded(
    command: &mut Command,
    timeout: Duration,
) -> (ProcessState, Option<Vec<u32>>) {
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::null());
    #[cfg(unix)]
    command.stdout(Stdio::piped());
    let Ok(mut child) = command.spawn() else {
        return (ProcessState::Unknown, None);
    };
    #[cfg(unix)]
    let mut output = child.stdout.take();
    #[cfg(unix)]
    if let Some(output) = &output {
        use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
        if fcntl_getfl(output)
            .and_then(|flags| fcntl_setfl(output, flags | OFlags::NONBLOCK))
            .is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            return (ProcessState::Unknown, None);
        }
    }
    let deadline = Instant::now() + timeout;
    #[cfg(unix)]
    let mut bytes = Vec::new();
    loop {
        #[cfg(unix)]
        if let Some(output) = output.as_mut() {
            let mut buffer = [0; 4096];
            match output.read(&mut buffer) {
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return (ProcessState::Unknown, None);
                }
            }
            if bytes.len() > 65536 {
                let _ = child.kill();
                let _ = child.wait();
                return (ProcessState::Unknown, None);
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                let ids = {
                    // The child has exited: drain only bytes already available, with the same fixed output bound.
                    if let Some(output) = output {
                        let _ = std::io::Read::take(output, 65536).read_to_end(&mut bytes);
                    }
                    Some(
                        String::from_utf8_lossy(&bytes)
                            .lines()
                            .filter_map(|line| line.trim().parse().ok())
                            .collect(),
                    )
                };
                #[cfg(not(unix))]
                let ids = None;
                return (
                    if status.success() {
                        ProcessState::Running
                    } else if status.code() == Some(1) {
                        ProcessState::Stopped
                    } else {
                        ProcessState::Unknown
                    },
                    if status.success() || status.code() == Some(1) {
                        ids
                    } else {
                        None
                    },
                );
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return (ProcessState::Unknown, None);
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return (ProcessState::TimedOut, None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
