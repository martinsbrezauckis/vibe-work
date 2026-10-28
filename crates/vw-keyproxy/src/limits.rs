//! Rate-limit headroom read from Mistral's answers.
//!
//! Mistral answers chat completions with five `x-ratelimit-*` headers (checked live;
//! `GET /v1/models` carries none). The proxy reads **only those five header names** from an upstream
//! answer, never a body and never any other header, parses each as an unsigned integer and offers
//! one [`RateLimits`] snapshot per answer that carried at least one of them to [`LimitsGate`] (latest wins; a field
//! missing from an answer is `None`, nothing is carried over from older answers).
//!
//! Depends on: axum's `http` types, tokio (the gate's one trailing call). Called by: `server` (after the upstream
//! answer arrived, before the body is streamed back). Invariants:
//! - values are never logged and never kept as text, only the parsed numbers;
//! - a header that appears more than once, is not plain ASCII digits (sign, space inside, unit,
//!   decimal) or does not fit `u64` is ignored: the field stays `None`;
//! - a remaining count above the limit of the same kind is not believed: both numbers of that kind are `None`
//!   (an upstream that misbehaves cannot make the app show more headroom than the limit);
//! - the owner's hook is called at most once per [`LIMITS_EVERY`] per proxy (one key at a time): answers in between
//!   leave only their newest snapshot, which one trailing call passes on (a burst of answers cannot flood the
//!   core's event broadcast);
//! - there is no monthly-quota header; none is invented.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::http::HeaderMap;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::server::LimitsHook;

/// Least time between two calls of the owner's rate-limit hook.
pub const LIMITS_EVERY: Duration = Duration::from_secs(1);

/// Headers read from an upstream answer (lower case, the only names the proxy looks at).
pub const LIMIT_TOKENS: &str = "x-ratelimit-limit-tokens-minute";
/// See [`LIMIT_TOKENS`].
pub const REMAINING_TOKENS: &str = "x-ratelimit-remaining-tokens-minute";
/// See [`LIMIT_TOKENS`].
pub const LIMIT_REQUESTS: &str = "x-ratelimit-limit-req-minute";
/// See [`LIMIT_TOKENS`].
pub const REMAINING_REQUESTS: &str = "x-ratelimit-remaining-req-minute";

/// What one upstream answer said about the per-minute headroom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimits {
    /// Tokens per minute the key may use (`x-ratelimit-limit-tokens-minute`).
    pub tokens_limit: Option<u64>,
    /// Tokens left in the current minute (`x-ratelimit-remaining-tokens-minute`).
    pub tokens_remaining: Option<u64>,
    /// Requests per minute (`x-ratelimit-limit-req-minute`).
    pub requests_limit: Option<u64>,
    /// Requests left in the current minute (`x-ratelimit-remaining-req-minute`).
    pub requests_remaining: Option<u64>,
    /// When the answer arrived, milliseconds since the Unix epoch.
    pub observed_at_ms: i64,
}

/// One header as a number: exactly one occurrence, ASCII digits only, fits `u64`.
fn number(headers: &HeaderMap, name: &str) -> Option<u64> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let text = value.to_str().ok()?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A limit and its remaining count as sent, or neither when the remaining count is above the limit (only a full
/// pair can be checked; half a pair is kept as sent).
fn consistent(limit: Option<u64>, remaining: Option<u64>) -> (Option<u64>, Option<u64>) {
    match (limit, remaining) {
        (Some(limit), Some(remaining)) if remaining > limit => (None, None),
        pair => pair,
    }
}

impl RateLimits {
    /// Reads the snapshot out of an upstream answer's headers; `None` when the answer carried none of
    /// the five usable values (so such an answer never replaces an earlier snapshot).
    #[must_use]
    pub fn from_headers(headers: &HeaderMap, observed_at_ms: i64) -> Option<Self> {
        let (tokens_limit, tokens_remaining) = consistent(
            number(headers, LIMIT_TOKENS),
            number(headers, REMAINING_TOKENS),
        );
        let (requests_limit, requests_remaining) = consistent(
            number(headers, LIMIT_REQUESTS),
            number(headers, REMAINING_REQUESTS),
        );
        let limits = Self {
            tokens_limit,
            tokens_remaining,
            requests_limit,
            requests_remaining,
            observed_at_ms,
        };
        let any = limits.tokens_limit.is_some()
            || limits.tokens_remaining.is_some()
            || limits.requests_limit.is_some()
            || limits.requests_remaining.is_some();
        any.then_some(limits)
    }
}

/// Passes snapshots to the owner's hook at most once per [`LIMITS_EVERY`], newest wins (see the module header).
pub(crate) struct LimitsGate {
    hook: LimitsHook,
    state: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    /// When the hook was last called.
    last: Option<Instant>,
    /// The newest snapshot not passed on yet.
    pending: Option<RateLimits>,
    /// The trailing call, armed while a snapshot waits (at most one; it ends by itself within one interval and is
    /// aborted by [`LimitsGate::close`]).
    timer: Option<JoinHandle<()>>,
}

impl LimitsGate {
    pub(crate) fn new(hook: LimitsHook) -> Arc<Self> {
        Arc::new(Self {
            hook,
            state: Mutex::default(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Calls the hook now when the last call is at least [`LIMITS_EVERY`] ago and none is waiting; else keeps
    /// `limits` as the newest waiting snapshot and arms the trailing call. Runs on the request task (a runtime is
    /// present); the hook itself is called without the lock held.
    pub(crate) fn offer(self: &Arc<Self>, limits: RateLimits) {
        let now = Instant::now();
        let mut state = self.lock();
        let free = state
            .last
            .is_none_or(|last| now.duration_since(last) >= LIMITS_EVERY);
        if free && state.timer.is_none() {
            state.last = Some(now);
            drop(state);
            (self.hook)(limits);
            return;
        }
        state.pending = Some(limits);
        if state.timer.is_none() {
            let due = state.last.map_or(now, |last| last + LIMITS_EVERY);
            let gate = Arc::clone(self);
            state.timer = Some(tokio::spawn(async move {
                tokio::time::sleep_until(due).await;
                gate.trailing();
            }));
        }
    }

    /// The trailing call: passes the newest waiting snapshot on.
    fn trailing(&self) {
        let mut state = self.lock();
        state.timer = None;
        state.last = Some(Instant::now());
        let pending = state.pending.take();
        drop(state);
        if let Some(limits) = pending {
            (self.hook)(limits);
        }
    }

    /// Stops a waiting trailing call (the proxy shuts down); its snapshot is dropped.
    pub(crate) fn close(&self) {
        let mut state = self.lock();
        state.pending = None;
        if let Some(timer) = state.timer.take() {
            timer.abort();
        }
    }
}

/// Milliseconds since the Unix epoch (0 when the clock is before it).
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
