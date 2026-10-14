//! Connection tuning: frame limit, timeouts, queue sizes, wedge rules.
//!
//! Depends on: `stderr_ring::StderrLimits`. Called by: callers building a connection (the
//! supervisor, tests).
//! Invariant: every value is bounded; there is no "wait forever" setting.

use std::time::Duration;

use crate::stderr_ring::{Redactions, StderrLimits};

/// Methods whose unanswered request means the server is wedged, not just slow.
///
/// `callback/result`: a malformed
/// answer gets no response within 30 s and Vibe 2.26.0 later exits with code 1. Treating the
/// silence as death lets the supervisor restart instead of hanging the turn.
pub const DEFAULT_WEDGE_METHODS: &[&str] = &["callback/result"];

/// Methods that need more than the ordinary 30 s request timeout.
///
/// `mcp_catalog/login` (and its alias `mcp/login`) answers only after the user finished the sign-in in
/// the browser: Vibe 2.26.0 waits up to 600 s for the OAuth callback (`core/auth/mcp_oauth.py:44`,
/// `_CALLBACK_TIMEOUT_SECONDS`). 660 s leaves Vibe the time to answer its own timeout.
pub const DEFAULT_LONG_METHODS: &[(&str, Duration)] = &[
    // The model proxy bounds compaction at 60 s; leave time for Vibe to persist its result
    // and answer with either the summary or the explicit proxy failure.
    ("session/compact", Duration::from_secs(90)),
    ("mcp_catalog/login", Duration::from_secs(660)),
    ("mcp/login", Duration::from_secs(660)),
];

#[cfg(test)]
mod tests {
    use super::{ConnectionConfig, DEFAULT_LONG_METHODS};
    use std::time::Duration;

    #[test]
    fn manual_compaction_waits_longer_than_the_model_proxy_deadline() {
        assert_eq!(
            DEFAULT_LONG_METHODS
                .iter()
                .find(|(method, _)| *method == "session/compact"),
            Some(&("session/compact", Duration::from_secs(90)))
        );
        let config = ConnectionConfig::default();
        assert!(
            config
                .method_timeouts
                .contains(&("session/compact".to_owned(), Duration::from_secs(90)))
        );
    }
}

/// Tuning for one connection. `Default` gives production values.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Longest accepted frame (content bytes, newline excluded). Longer frames are discarded and
    /// reported as a gap. Default 64 MiB: a long session's `session/read` carries its history.
    pub max_frame_bytes: usize,
    /// Timeout for requests that do not set their own. Default 30 s.
    pub default_timeout: Duration,
    /// Timeout for one stdin write; expiry means the server stopped reading (dead). Default 5 s,
    /// the same bound ShellX uses (`provider_sessions.rs:54 @ e38c2b5c`).
    pub write_timeout: Duration,
    /// Frames queued for the writer before senders wait. Default 1024.
    pub outbound_queue: usize,
    /// Items buffered per subscriber; a subscriber that falls this far behind is dropped.
    /// Default 4096.
    pub subscriber_capacity: usize,
    /// After stdout EOF, how long to wait for the exit status and the last stderr lines before
    /// declaring the server dead. Default 2 s.
    pub exit_grace: Duration,
    /// How many stderr lines a death report carries. Default 40.
    pub death_stderr_lines: usize,
    /// stderr ring limits.
    pub stderr: StderrLimits,
    /// Secret values (the API key) replaced in captured stderr before it is stored.
    pub stderr_redactions: Redactions,
    /// Methods whose timeout declares the server dead (see [`DEFAULT_WEDGE_METHODS`]).
    pub wedge_methods: Vec<String>,
    /// Methods with their own default timeout instead of `default_timeout` (see
    /// [`DEFAULT_LONG_METHODS`]); a request's own `RequestOptions::timeout` still wins.
    pub method_timeouts: Vec<(String, Duration)>,
    /// Unanswered requests allowed at once on one channel of a multiplexed process; more wait for a
    /// free slot (inside their own timeout). Default 64. Why: Vibe's `multiplex.py` gives each channel
    /// an inbox of 256 client messages and closes a channel whose inbox is full
    /// (`_CHANNEL_INBOX_SIZE`); a channel's server handles requests concurrently, so the inbox only
    /// fills when far more frames arrive than it can take in. Plain connections have no such bound.
    pub channel_max_in_flight: usize,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            max_frame_bytes: 64 * 1024 * 1024,
            default_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(5),
            outbound_queue: 1024,
            subscriber_capacity: 4096,
            exit_grace: Duration::from_secs(2),
            death_stderr_lines: 40,
            stderr: StderrLimits::default(),
            stderr_redactions: Redactions::default(),
            wedge_methods: DEFAULT_WEDGE_METHODS
                .iter()
                .map(|m| (*m).to_owned())
                .collect(),
            method_timeouts: DEFAULT_LONG_METHODS
                .iter()
                .map(|(m, t)| ((*m).to_owned(), *t))
                .collect(),
            channel_max_in_flight: 64,
        }
    }
}

/// What a request timeout means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutPolicy {
    /// The request fails with `RpcError::Timeout`; the connection stays up.
    Fail,
    /// The server is declared dead (`DeathCause::Wedged`).
    ServerDead,
}

/// Per-request overrides.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestOptions {
    /// Timeout; `None` uses the connection default.
    pub timeout: Option<Duration>,
    /// Timeout meaning; `None` uses the wedge list (wedge methods are fatal, others fail).
    pub on_timeout: Option<TimeoutPolicy>,
}

impl RequestOptions {
    /// Options with only a timeout set.
    #[must_use]
    pub fn timeout(timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
            on_timeout: None,
        }
    }
}
