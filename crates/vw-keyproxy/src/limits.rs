//! Rate-limit headroom read from Mistral's answers.
//!
//! Mistral answers chat completions with five `x-ratelimit-*` headers (checked live;
//! `GET /v1/models` carries none). The proxy reads **only those five header names** from an upstream
//! answer, never a body and never any other header, parses each as an unsigned integer and publishes
//! one [`RateLimits`] snapshot per answer that carried at least one of them (latest wins; a field
//! missing from an answer is `None`, nothing is carried over from older answers).
//!
//! Depends on: axum's `http` types only. Called by: `server` (after the upstream answer arrived,
//! before the body is streamed back). Invariants:
//! - values are never logged and never kept as text, only the parsed numbers;
//! - a header that appears more than once, is not plain ASCII digits (sign, space inside, unit,
//!   decimal) or does not fit `u64` is ignored: the field stays `None`;
//! - there is no monthly-quota header; none is invented.

use axum::http::HeaderMap;

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

impl RateLimits {
    /// Reads the snapshot out of an upstream answer's headers; `None` when the answer carried none of
    /// the five usable values (so such an answer never replaces an earlier snapshot).
    #[must_use]
    pub fn from_headers(headers: &HeaderMap, observed_at_ms: i64) -> Option<Self> {
        let limits = Self {
            tokens_limit: number(headers, LIMIT_TOKENS),
            tokens_remaining: number(headers, REMAINING_TOKENS),
            requests_limit: number(headers, LIMIT_REQUESTS),
            requests_remaining: number(headers, REMAINING_REQUESTS),
            observed_at_ms,
        };
        let any = limits.tokens_limit.is_some()
            || limits.tokens_remaining.is_some()
            || limits.requests_limit.is_some()
            || limits.requests_remaining.is_some();
        any.then_some(limits)
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
