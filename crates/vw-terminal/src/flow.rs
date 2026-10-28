//! A terminal's output side: the scrollback, the one attached viewer, and acknowledgement-based flow
//! control between them.
//!
//! Depends on: `ring`, `text`, `types`, `viewer`. Called by: `terminal` (the reader thread pushes output), the
//! manager (attach, ack, detach, reads).
//! Invariants:
//! - Every byte the program prints goes into the scrollback first, so reads and re-attaches see it
//!   whether or not a viewer is attached.
//! - At most one viewer: a new attach replaces the old one (a reloaded window just attaches again).
//!   The viewer gets the scrollback replay, then live bytes, in order, with no gap and no duplicate
//!   (both happen under the same lock as the scrollback append).
//! - Live output is batched at 64 KiB or 8 ms after its first pending byte. Output after idle goes
//!   straight through. A separate worker flushes the tail even while the reader waits in the PTY.
//! - Flow control: while the viewer has `high_water` or more unacknowledged bytes (including a
//!   pending batch), `push` flushes that batch and blocks the reader thread, so the PTY fills and
//!   the program blocks on its next write. A viewer that
//!   does not acknowledge within `ack_timeout` is detached and the terminal goes back to buffering:
//!   a frozen or closed window never stops a program forever.
//! - Without a viewer `push` never blocks (hidden sessions keep running; scrollback is bounded).
//! - The lock is a std mutex held only for in-memory work and the viewer's non-blocking send.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use std::{io, thread};

use tracing::warn;

use crate::ring::Ring;
use crate::text::{strip_ansi, utf8_window};
use crate::types::{AttachInfo, OutputRead, ViewerSink};
use crate::viewer::Viewer;

/// How far into a truncated scrollback a replay looks for a line start (so a replay does not
/// begin in the middle of an escape sequence when it can avoid it).
const REPLAY_ALIGN: usize = 4096;

struct State {
    ring: Ring,
    viewer: Option<Viewer>,
    next_viewer: u64,
    /// Set when the terminal is closing: `push` stops waiting for acknowledgements.
    closed: bool,
}

/// Shared with the flush worker, which owns no terminal or process handles.
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn flush_loop(&self) {
        let mut state = self.lock();
        while !state.closed {
            let deadline = state.viewer.as_ref().and_then(|viewer| viewer.deadline);
            state = if let Some(deadline) = deadline {
                let now = Instant::now();
                if now >= deadline {
                    if state.viewer.as_mut().is_some_and(|v| !v.flush(now)) {
                        state.viewer = None;
                        self.changed.notify_all();
                    }
                    continue;
                }
                self.changed
                    .wait_timeout(state, deadline - now)
                    .map_or_else(|poison| poison.into_inner().0, |(guard, _)| guard)
            } else {
                self.changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner)
            };
        }
    }
}

/// The output side of one terminal (shared by every shell generation of that terminal).
pub(crate) struct Output {
    shared: Arc<Shared>,
    high_water: u64,
    ack_timeout: Duration,
}

impl Output {
    pub(crate) fn new(
        scrollback: usize,
        high_water: u64,
        ack_timeout: Duration,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                ring: Ring::new(scrollback),
                viewer: None,
                next_viewer: 1,
                closed: false,
            }),
            changed: Condvar::new(),
        });
        let worker = shared.clone();
        thread::Builder::new()
            .name("vw-term-flush".into())
            .spawn(move || worker.flush_loop())?;
        Ok(Self {
            shared,
            high_water: high_water.max(1),
            ack_timeout,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.lock()
    }

    /// Appends program output, forwards it to the viewer, and blocks while the viewer is behind.
    /// Called only from the reader thread.
    pub(crate) fn push(&self, data: &[u8]) {
        let mut state = self.lock();
        state.ring.push(data);
        let mut wake = false;
        let gone = state.viewer.as_mut().is_some_and(|viewer| {
            let was_pending = viewer.deadline.is_some();
            let alive = viewer.push(data, Instant::now());
            wake = !was_pending && viewer.deadline.is_some();
            // The viewer must receive pending bytes before the reader waits for their ack.
            !alive || (viewer.outstanding() >= self.high_water && !viewer.flush(Instant::now()))
        });
        if gone {
            state.viewer = None;
        }
        if wake {
            self.shared.changed.notify_all();
        }
        let deadline = Instant::now() + self.ack_timeout;
        loop {
            let behind = !state.closed
                && state
                    .viewer
                    .as_ref()
                    .is_some_and(|v| v.unacked >= self.high_water);
            if !behind {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                warn!(
                    "terminal viewer acknowledged nothing for {:?}; detached, output keeps buffering",
                    self.ack_timeout
                );
                state.viewer = None;
                return;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, deadline - now)
                .map_or_else(|poison| poison.into_inner().0, |(guard, _)| guard);
        }
    }

    /// Adds bytes written by the app itself (a restart marker) without flow control.
    pub(crate) fn note(&self, data: &[u8]) {
        let mut state = self.lock();
        state.ring.push(data);
        let gone = state.viewer.as_mut().is_some_and(|viewer| {
            let now = Instant::now();
            !viewer.flush(now) || !viewer.send(data, now)
        });
        if gone {
            state.viewer = None;
        }
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Attaches `sink` as the viewer (replacing any other), replays the scrollback into it, and
    /// returns where the replay started and ended.
    pub(crate) fn attach(&self, sink: ViewerSink) -> AttachInfo {
        let mut state = self.lock();
        let ring = &state.ring;
        let (mut start, mut replay) = ring.read(ring.start(), ring.len());
        if start > 0 {
            // History was dropped: begin after the first line break when one is near.
            if let Some(nl) = replay.iter().take(REPLAY_ALIGN).position(|b| *b == b'\n') {
                replay.drain(..=nl);
                start += nl as u64 + 1;
            }
        }
        let end = start + replay.len() as u64;
        let id = state.next_viewer;
        state.next_viewer += 1;
        let mut viewer = Viewer::new(id, sink);
        let alive = !state.closed && viewer.send(&replay, Instant::now());
        state.viewer = alive.then_some(viewer);
        drop(state);
        // A replaced viewer may have had a reader waiting on it.
        self.shared.changed.notify_all();
        AttachInfo {
            viewer: id,
            start,
            end,
        }
    }

    /// The viewer `viewer` has processed `bytes` more bytes.
    pub(crate) fn ack(&self, viewer: u64, bytes: u64) {
        let mut state = self.lock();
        if let Some(v) = state.viewer.as_mut().filter(|v| v.id == viewer) {
            v.unacked = v.unacked.saturating_sub(bytes);
        }
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Detaches `viewer` (no-op if another viewer replaced it).
    pub(crate) fn detach(&self, viewer: u64) {
        let mut state = self.lock();
        if state.viewer.as_ref().is_some_and(|v| v.id == viewer) {
            state.viewer = None;
        }
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Stops flow control for good (the terminal is closing) and drops the viewer.
    pub(crate) fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.viewer = None;
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Offset just past the newest byte.
    pub(crate) fn end(&self) -> u64 {
        let state = self.lock();
        let end = state.ring.end();
        if let Some(viewer) = &state.viewer {
            viewer.report(end);
        }
        end
    }

    /// Up to `max` bytes from offset `from` (`None`: the newest `max` bytes) as text; `plain`
    /// removes escape sequences.
    pub(crate) fn read(&self, from: Option<u64>, max: usize, plain: bool) -> OutputRead {
        // At least one whole character (4 bytes) so a read always makes progress.
        let max = max.max(4);
        let state = self.lock();
        let ring = &state.ring;
        let wanted = from.unwrap_or_else(|| ring.end().saturating_sub(max as u64));
        let truncated = wanted < ring.start();
        let (offset, bytes) = ring.read(wanted, max);
        drop(state);
        let (skip, keep) = utf8_window(&bytes);
        let slice = &bytes[skip..skip + keep];
        let text = String::from_utf8_lossy(slice);
        OutputRead {
            start: offset + skip as u64,
            end: offset + (skip + keep) as u64,
            truncated,
            text: if plain {
                strip_ansi(&text)
            } else {
                text.into_owned()
            },
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // Wake the sleeping worker even when a manager is dropped without an explicit close.
        self.close();
    }
}
