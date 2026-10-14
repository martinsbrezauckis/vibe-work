//! A per-session allowance for `session/read` resyncs, so a session whose state will not settle
//! cannot make the core hammer Vibe.
//!
//! Depends on: std only. Called by: `pump` (before starting a resync); tests.
//! Replaying a stale event onto every fresh read after `session/resume` can
//! make the pump read one session thousands of times in seconds. The root cause is fixed in
//! `vw-session` (`SessionEntry::replace_state`); this is the safety net for causes not seen yet.
//! Invariants:
//! - A healthy session needs one or two resyncs after a gap, far below the burst allowance.
//! - Past `burst` resyncs within `window`, the session is paused for `pause`; the call that starts the
//!   pause says so (`first`), so the alert is raised once, not on every refused attempt.
//! - Pure bookkeeping over an injected clock value: no timers, no tasks.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// The allowance for every session.
#[derive(Debug, Clone)]
pub struct ResyncBudget {
    burst: usize,
    window: Duration,
    pause: Duration,
    recent: BTreeMap<String, VecDeque<Instant>>,
    paused_until: BTreeMap<String, Instant>,
}

/// The budget's answer to one request for a resync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    /// Go ahead.
    Allowed,
    /// Not now. `first` is true only for the request that started the pause.
    Paused {
        /// This request started the pause.
        first: bool,
    },
}

impl ResyncBudget {
    /// An allowance of `burst` resyncs per `window` per session, then a pause of `pause`.
    #[must_use]
    pub fn new(burst: usize, window: Duration, pause: Duration) -> Self {
        Self {
            burst,
            window,
            pause,
            recent: BTreeMap::new(),
            paused_until: BTreeMap::new(),
        }
    }

    /// The production values: 8 resyncs per 5 s per session, then 30 s of pause.
    #[must_use]
    pub fn standard() -> Self {
        Self::new(8, Duration::from_secs(5), Duration::from_secs(30))
    }

    /// Asks to start a resync of `session_id` at `now`; counts it when allowed.
    pub fn spend(&mut self, session_id: &str, now: Instant) -> Budget {
        if let Some(until) = self.paused_until.get(session_id) {
            if now < *until {
                return Budget::Paused { first: false };
            }
            self.paused_until.remove(session_id);
            self.recent.remove(session_id);
        }
        let times = self.recent.entry(session_id.to_owned()).or_default();
        while times
            .front()
            .is_some_and(|t| now.duration_since(*t) > self.window)
        {
            times.pop_front();
        }
        if times.len() >= self.burst {
            self.paused_until
                .insert(session_id.to_owned(), now + self.pause);
            return Budget::Paused { first: true };
        }
        times.push_back(now);
        Budget::Allowed
    }
}
