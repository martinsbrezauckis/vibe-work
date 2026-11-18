//! Process-exit notices: a callback runs as soon as a watched process exits, so a caller that keeps per-process
//! state (the App preview's allowed chat-server ports) can drop it at once instead of at its next periodic scan.
//!
//! Depends on: `os` (identity, for the pid-reuse check), nix `event` (macOS kqueue). Called by: vw-core-api (the
//! chats' servers that a process scan attributed).
//! Invariants:
//! - macOS only ([`ExitWatch::start`] is `None` elsewhere): there `WebKit` sends loopback address literals past the
//!   preview proxy, so static content rules are the only control on them and must follow a server's exit quickly.
//!   Linux and Windows send every preview request through the proxy, which checks the server at each request.
//! - A notice may be early or extra, never missing: a process that has already exited when it is watched, or whose
//!   pid now belongs to another process (start token differs), is reported at once; a reused pid's later exit only
//!   causes one more notice. Callers treat a notice as "re-check now", never as proof of anything.
//! - Each pid is registered once while it is watched; the kernel removes the registration when the exit fires.

use std::sync::Arc;

use crate::types::ProcessRef;

/// What runs on an exit notice (the exited pid). Called on the watcher's own thread: keep it short.
pub type OnExit = Arc<dyn Fn(u32) + Send + Sync>;

/// A running exit watcher; dropping it ends its thread.
pub struct ExitWatch {
    #[cfg(target_os = "macos")]
    inner: mac::Watcher,
}

impl std::fmt::Debug for ExitWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExitWatch").finish_non_exhaustive()
    }
}

impl ExitWatch {
    /// Starts the watcher thread (`None`: this system has no watcher, or the kernel queue could not be made).
    #[must_use]
    pub fn start(on_exit: OnExit) -> Option<Self> {
        #[cfg(target_os = "macos")]
        return mac::Watcher::start(on_exit).map(|inner| Self { inner });
        #[cfg(not(target_os = "macos"))]
        return {
            drop(on_exit);
            None
        };
    }

    /// Watches each of `processes` that is not watched yet. One that already exited, or whose pid now belongs to
    /// another process, is reported at once (on the caller's thread).
    pub fn watch(&self, processes: &[ProcessRef]) {
        #[cfg(target_os = "macos")]
        self.inner.watch(processes);
        #[cfg(not(target_os = "macos"))]
        let _ = processes;
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex, PoisonError};

    use nix::errno::Errno;
    use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent, Kqueue};

    use super::OnExit;
    use crate::types::ProcessRef;

    /// The ident of the user event that ends the thread.
    const STOP: usize = 0;

    struct Shared {
        queue: Kqueue,
        watched: Mutex<HashSet<u32>>,
        on_exit: OnExit,
    }

    impl Shared {
        fn exited(&self, pid: u32) {
            self.watched
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&pid);
            (self.on_exit)(pid);
        }
    }

    pub(super) struct Watcher {
        shared: Arc<Shared>,
    }

    fn user_event(flags: EvFlags, notes: FilterFlag) -> KEvent {
        KEvent::new(STOP, EventFilter::EVFILT_USER, flags, notes, 0, 0)
    }

    impl Watcher {
        pub(super) fn start(on_exit: OnExit) -> Option<Self> {
            let queue = match Kqueue::new() {
                Ok(queue) => queue,
                Err(error) => {
                    tracing::warn!(%error, "process exit watch is off: no kernel queue");
                    return None;
                }
            };
            let stop = user_event(EvFlags::EV_ADD | EvFlags::EV_CLEAR, FilterFlag::empty());
            if let Err(error) = queue.kevent(&[stop], &mut [], None) {
                tracing::warn!(%error, "process exit watch is off: its stop event was refused");
                return None;
            }
            let shared = Arc::new(Shared {
                queue,
                watched: Mutex::new(HashSet::new()),
                on_exit,
            });
            let thread = Arc::clone(&shared);
            if let Err(error) = std::thread::Builder::new()
                .name("vw-exit-watch".to_owned())
                .spawn(move || run(&thread))
            {
                tracing::warn!(%error, "process exit watch is off: its thread did not start");
                return None;
            }
            Some(Self { shared })
        }

        pub(super) fn watch(&self, processes: &[ProcessRef]) {
            for process in processes {
                let fresh = self
                    .shared
                    .watched
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(process.pid);
                let Ok(ident) = usize::try_from(process.pid) else {
                    continue;
                };
                if !fresh {
                    continue;
                }
                let exit = KEvent::new(
                    ident,
                    EventFilter::EVFILT_PROC,
                    EvFlags::EV_ADD | EvFlags::EV_ONESHOT,
                    FilterFlag::NOTE_EXIT,
                    0,
                    0,
                );
                match self.shared.queue.kevent(&[exit], &mut [], None) {
                    // Registered: the pid must still be the scanned process (checked after registering, so an
                    // exit in between is either seen by the queue or caught here).
                    Ok(_) if crate::os::identity(process.pid) == Some(*process) => {}
                    // The pid is another process now, or (ESRCH) no process: the scanned one already exited.
                    Ok(_) | Err(Errno::ESRCH) => self.shared.exited(process.pid),
                    Err(error) => {
                        tracing::debug!(%error, pid = process.pid, "process exit not watched");
                        self.shared
                            .watched
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(&process.pid);
                    }
                }
            }
        }
    }

    impl Drop for Watcher {
        fn drop(&mut self) {
            let trigger = user_event(EvFlags::empty(), FilterFlag::NOTE_TRIGGER);
            if let Err(error) = self.shared.queue.kevent(&[trigger], &mut [], None) {
                tracing::debug!(%error, "process exit watch did not stop");
            }
        }
    }

    fn run(shared: &Shared) {
        let empty = KEvent::new(
            0,
            EventFilter::EVFILT_USER,
            EvFlags::empty(),
            FilterFlag::empty(),
            0,
            0,
        );
        let mut events = [empty; 16];
        loop {
            let count = match shared.queue.kevent(&[], &mut events, None) {
                Ok(count) => count,
                Err(Errno::EINTR) => continue,
                Err(error) => {
                    tracing::warn!(%error, "process exit watch stopped");
                    return;
                }
            };
            for event in &events[..count] {
                match event.filter() {
                    Ok(EventFilter::EVFILT_USER) => return,
                    Ok(EventFilter::EVFILT_PROC)
                        if event.fflags().contains(FilterFlag::NOTE_EXIT) =>
                    {
                        if let Ok(pid) = u32::try_from(event.ident()) {
                            shared.exited(pid);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::ExitWatch;

    fn watcher() -> (ExitWatch, mpsc::Receiver<u32>) {
        let (send, receive) = mpsc::channel();
        let send = std::sync::Mutex::new(send);
        let watch = ExitWatch::start(std::sync::Arc::new(move |pid| {
            let _ = send.lock().unwrap().send(pid);
        }))
        .expect("macOS has an exit watcher");
        (watch, receive)
    }

    /// PREDICT: a chat server's exit reaches the caller within tens of milliseconds, not at a periodic scan.
    #[test]
    fn a_watched_process_exit_is_reported_at_once() {
        let (watch, exits) = watcher();
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let process = crate::os::identity(child.id()).expect("the child runs");
        watch.watch(&[process]);
        assert!(
            exits.recv_timeout(Duration::from_millis(300)).is_err(),
            "no notice while it runs"
        );
        child.kill().unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(2)),
            Ok(child.id()),
            "the exit notice"
        );
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
        let _ = child.wait();
    }

    /// A process that is gone before it is watched, or a pid that now belongs to another process, is reported
    /// at once: a notice is never missed because the exit came first.
    #[test]
    fn a_process_gone_before_the_watch_is_reported_at_once() {
        let (watch, exits) = watcher();
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let process = crate::os::identity(child.id());
        let _ = child.wait();
        let gone = process.unwrap_or(crate::types::ProcessRef {
            pid: child.id(),
            start: crate::types::StartToken(1),
        });
        watch.watch(&[gone]);
        assert_eq!(exits.recv_timeout(Duration::from_secs(1)), Ok(child.id()));
        let me = crate::os::identity(std::process::id()).unwrap();
        let reused = crate::types::ProcessRef {
            pid: me.pid,
            start: crate::types::StartToken(me.start.0.wrapping_add(1)),
        };
        watch.watch(&[reused]);
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(1)),
            Ok(me.pid),
            "another process on the pid"
        );
    }
}
