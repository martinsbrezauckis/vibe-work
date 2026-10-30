//! Runs the app's shutdown when the process is about to end (macOS Cmd+Q, the last window closed, an updater
//! restart), on the calling thread, with a bound that holds even when the async runtime is stuck.
//!
//! Depends on: tokio (the runtime handle only). Called by: [`crate::Host::shutdown_at_exit`]; the app calls that
//! from its exit event on the main thread.
//!
//! Why not `Handle::block_on`: the caller is the UI thread, held here until the shutdown ends, so a runtime task
//! that waits for the UI thread blocks its worker for good. On macOS the reply to a window command that finishes
//! during the quit is such a task (Tauri hands the response to the web view from the worker, which then waits for
//! the main run loop); window getters are others. In tokio's multi-thread runtime only a parking worker polls the
//! I/O and time driver; when the worker that held it runs such a task, the other workers stay asleep and no timer,
//! no I/O and no child-exit signal is handled any more. Every limit inside the shutdown is a tokio timer, so the quit then never ends, and the exited Vibe
//! server stays an unreaped zombie.
//! Invariants:
//! - The future is polled on the calling thread (inside the runtime's context), exactly where `block_on` polled
//!   it: work the shutdown does on the UI thread still runs inline there.
//! - Between polls the thread waits on its own park with a real-time timeout, never on a tokio timer.
//! - While it waits, an empty task is scheduled on the runtime every [`NUDGE`]. Scheduling from outside the
//!   runtime wakes a sleeping worker, which takes over the driver when it parks again; so timers, I/O and child
//!   reaping go on while one worker stays blocked.
//! - After `limit` the future is dropped unfinished and `None` is returned: the caller then stops what must not
//!   outlive the app by synchronous means.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::Thread;
use std::time::{Duration, Instant};

use tokio::runtime::Handle;

/// How often a waiting exit schedules an empty task to wake a runtime worker.
pub const NUDGE: Duration = Duration::from_millis(100);

/// Wakes the thread that polls the exit future.
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Polls `work` to completion on this thread within `limit` (see the module doc). `None`: `limit` passed first and
/// `work` was dropped unfinished. Must not be called from inside a runtime worker.
pub fn finish<F: Future>(runtime: &Handle, work: F, limit: Duration) -> Option<F::Output> {
    let _context = runtime.enter();
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut work = pin!(work);
    let deadline = Instant::now() + limit;
    let mut next_nudge = Instant::now() + NUDGE;
    loop {
        if let Poll::Ready(output) = work.as_mut().poll(&mut cx) {
            return Some(output);
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        if now >= next_nudge {
            // Detached: the task does nothing; scheduling it is the point.
            drop(runtime.spawn(async {}));
            next_nudge = now + NUDGE;
        }
        std::thread::park_timeout(deadline.min(next_nudge).saturating_duration_since(now));
    }
}

/// The whole quit may take this long before the Vibe server is killed instead of stopped: well above the
/// supervisor's own bounds (5 s for stdin EOF, then kill and reap), far below what a user waits before Force Quit.
pub const EXIT_LIMIT: Duration = Duration::from_secs(20);

impl crate::Host {
    /// The app's quit: [`crate::Host::shutdown`] through [`finish`] on the calling thread, at most `limit`. When it
    /// does not finish, the Vibe server's tree and its watchdog are killed synchronously
    /// ([`vw_core_api::Core::stop_server_now`]), so nothing of the app outlives it. Returns whether the shutdown
    /// finished.
    pub fn shutdown_at_exit(&self, runtime: &Handle, limit: Duration) -> bool {
        if finish(runtime, self.shutdown(), limit).is_some() {
            return true;
        }
        let killed = self.core().is_some_and(vw_core_api::Core::stop_server_now);
        tracing::warn!(
            limit_ms = limit.as_millis(),
            server_killed = killed,
            "quit: the shutdown did not finish in time; the Vibe server was killed"
        );
        false
    }
}
