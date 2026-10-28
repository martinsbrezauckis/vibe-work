//! Linux: keeps the descriptors this process inherited from reaching the next program it starts.
//!
//! Why: an `AppImage`'s runtime mounts the image (FUSE) and runs the app with an open descriptor that keeps that mount
//! alive while it is open, and that descriptor is not close-on-exec. Every program the app starts inherits it. After
//! an in-app update the app restarts by starting the NEW `AppImage` (Tauri `process::restart`, `Command::spawn`), so the
//! new app held the OLD image's descriptor: the old image stayed mounted (`/tmp/.mount_*`, its FUSE process) and its
//! file stayed on disk, deleted, until the updated app quit (107 MB). Marking every descriptor above stderr
//! close-on-exec right before that restart means the new process starts with only stdin, stdout and stderr.
//!
//! Invariants: only descriptors 3 and up; the flag changes nothing for this process itself (it only stops
//! inheritance at the next exec), so it is safe to call while other threads still use them; descriptors that close
//! meanwhile are skipped. Call it only when this process is about to start a program and exit (the restart).
//! Depends on: rustix (fcntl), `/proc/self/fd`. Called by: `app` (`updater.rs`, the restart after an update).

use std::os::fd::{BorrowedFd, RawFd};

use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};

/// Marks every open descriptor above 2 close-on-exec; returns how many were changed. Nothing changes when
/// `/proc/self/fd` cannot be read.
#[must_use]
pub fn stop_descriptor_inheritance() -> usize {
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return 0;
    };
    // Collect first: the listing's own descriptor is open (close-on-exec) while it is read.
    let fds: Vec<RawFd> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .filter(|fd| *fd > 2)
        .collect();
    let mut changed = 0;
    for raw in fds {
        // SAFETY: `raw` was listed as open by /proc/self/fd a moment ago and is only borrowed for two fcntl calls
        // that never close or read it. If it was closed meanwhile, fcntl fails with EBADF and it is skipped; if the
        // number was reused, the new descriptor gets close-on-exec, which only stops its inheritance (intended).
        let fd = unsafe { BorrowedFd::borrow_raw(raw) };
        let Ok(flags) = fcntl_getfd(fd) else {
            continue;
        };
        if !flags.contains(FdFlags::CLOEXEC) && fcntl_setfd(fd, flags | FdFlags::CLOEXEC).is_ok() {
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd;
    use std::process::Command;

    use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};

    use super::stop_descriptor_inheritance;

    /// Whether a child started now sees descriptor `fd` (what Tauri's restart does: a plain `Command::spawn`).
    fn child_sees(fd: i32) -> bool {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("test -e /proc/self/fd/{fd}"))
            .status()
            .unwrap()
            .success()
    }

    #[test]
    fn an_inherited_descriptor_no_longer_reaches_the_next_program() {
        // Stand-in for the AppImage runtime's keep-alive descriptor: a pipe without close-on-exec.
        let (reader, _writer) = std::io::pipe().unwrap();
        fcntl_setfd(&reader, FdFlags::empty()).unwrap();
        let fd = reader.as_raw_fd();
        assert!(
            child_sees(fd),
            "precondition: a child inherits a descriptor without close-on-exec"
        );
        assert!(stop_descriptor_inheritance() >= 1);
        assert!(fcntl_getfd(&reader).unwrap().contains(FdFlags::CLOEXEC));
        assert!(!child_sees(fd), "the child must not inherit it any more");
        // This process still uses it normally.
        assert!(fcntl_getfd(&reader).is_ok());
        assert_eq!(
            stop_descriptor_inheritance(),
            0,
            "a second call finds nothing left to change"
        );
    }
}
