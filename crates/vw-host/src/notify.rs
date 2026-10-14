//! The UI's request to show an OS notification, with the checks that make it safe to pass to the
//! OS: bounded text, a plain session id, and a rate cap.
//!
//! Depends on: vw-platform (`NotificationRequest`). Called by: [`crate::Host::notify`].
//! Invariants:
//! - The decision to notify (level, focus, open session) is the UI's (`ui/src/state/notify.ts`);
//!   this module only refuses abuse: text is clipped, control characters are dropped, an id that is
//!   not plain text is refused, and more than [`BURST`] notifications within [`WINDOW`] are dropped.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use vw_platform::NotificationRequest;

const TITLE_MAX: usize = 120;
const BODY_MAX: usize = 300;
const ID_MAX: usize = 128;
/// Notifications allowed per [`WINDOW`].
pub const BURST: usize = 8;
/// The rate-cap window.
pub const WINDOW: Duration = Duration::from_secs(10);

/// What the UI sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotifyRequest {
    /// Session the notification is about; a click opens it.
    pub session_id: String,
    /// In-app history entry to select when clicked; absent for older callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_id: Option<u32>,
    /// Title line.
    pub title: String,
    /// Body text.
    pub body: String,
}

/// Why a request was not shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotifyRefusal {
    /// The session id is empty, too long or not plain text.
    #[error("the session id is not valid")]
    BadSession,
    /// Too many notifications in a short time.
    #[error("too many notifications; this one was dropped")]
    RateLimited,
}

fn clean(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// Validates `request` and builds the OS notification, or says why not.
///
/// # Errors
/// [`NotifyRefusal`].
pub(crate) fn prepare(
    request: &NotifyRequest,
    limiter: &RateLimit,
    now: Instant,
) -> Result<NotificationRequest, NotifyRefusal> {
    let id_ok = !request.session_id.is_empty()
        && request.session_id.chars().count() <= ID_MAX
        && request
            .session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
    if !id_ok {
        return Err(NotifyRefusal::BadSession);
    }
    if !limiter.allow(now) {
        return Err(NotifyRefusal::RateLimited);
    }
    Ok(NotificationRequest {
        title: clean(&request.title, TITLE_MAX),
        body: clean(&request.body, BODY_MAX),
    })
}

/// Sliding-window counter.
#[derive(Default)]
pub(crate) struct RateLimit {
    times: Mutex<VecDeque<Instant>>,
}

impl RateLimit {
    pub(crate) fn allow(&self, now: Instant) -> bool {
        let mut times = self.times.lock().unwrap_or_else(PoisonError::into_inner);
        while times
            .front()
            .is_some_and(|t| now.duration_since(*t) >= WINDOW)
        {
            times.pop_front();
        }
        if times.len() >= BURST {
            return false;
        }
        times.push_back(now);
        true
    }
}
