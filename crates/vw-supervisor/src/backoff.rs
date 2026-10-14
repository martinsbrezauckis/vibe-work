//! Restart backoff: exponential with a cap, a give-up limit, and a reset after a stable run.
//!
//! Depends on: std. Called by: `supervisor`.
//! Invariant: the delay never exceeds `max`, and restarts stop after `max_attempts` consecutive
//! failures (no endless crash loop burning CPU or API calls).

use std::time::Duration;

/// Restart policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// Delay before the first restart.
    pub initial: Duration,
    /// Multiplier per further attempt (integer; 2 doubles).
    pub factor: u32,
    /// Largest delay.
    pub max: Duration,
    /// Restarts without a stable run in between after which the supervisor gives up (state
    /// `Failed`); a failed start counts as one.
    pub max_attempts: u32,
    /// A server that ran at least this long resets the attempt counter when it dies.
    pub stable_after: Duration,
}

impl Default for Backoff {
    /// 250 ms, doubling, capped at 30 s, give up after 6 attempts, reset after 60 s up.
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(250),
            factor: 2,
            max: Duration::from_secs(30),
            max_attempts: 6,
            stable_after: Duration::from_secs(60),
        }
    }
}

impl Backoff {
    /// Delay before restart attempt `attempt` (1-based): `initial * factor^(attempt-1)`, capped.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(1);
        let multiplier = self.factor.max(1).checked_pow(exponent).unwrap_or(u32::MAX);
        self.initial.saturating_mul(multiplier).min(self.max)
    }
}
