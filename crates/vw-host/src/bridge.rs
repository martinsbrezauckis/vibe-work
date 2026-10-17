//! `UiBridge`: the broker between the Debug API's `ui/*` routes and the UI that executes them.
//!
//! A `ui/*` route (select, click, read, open the side panel) acts on UI state that
//! lives in the webview or in a browser tab running the dev UI. The route handler runs in Rust and
//! cannot touch it, so it parks a [`UiRequest`] here and waits; a UI client takes the request with
//! [`UiBridge::next_request`] (the Tauri command `ui_control_next`, or `POST ui/ui-client/poll`
//! for the browser dev UI), runs it and answers with [`UiBridge::reply`].
//!
//! Depends on: vw-core-api (`CoreError`), vw-debug-api (`UiRoutes`), tokio. Called by: [`crate::Host`]
//! (registers the routes), `app` (the two Tauri commands), the private test host.
//! Invariants:
//! - A request is answered at most once and a reply for an unknown or finished id is reported as
//!   not accepted, never applied.
//! - No UI client attached means the request fails with `timeout` after the bridge's timeout and
//!   leaves nothing behind (queue and pending table are cleaned).
//! - Only the newest poll may take queued work. A poll left behind by a reloaded or recreated
//!   window returns empty when the new window polls, even if its transport has not disconnected.
//! - The bridge carries only what the route and the UI put in it; it never reaches Vibe, so it
//!   cannot be a path around the gate. A route that needs Vibe must go through `Core::execute`.
//! - **Capability:** every request carries a fresh random `capability` (256 bits
//!   from the OS RNG) that only the UI client taking the request sees. While the UI serves it, the commands it
//!   runs for it pass that capability ([`crate::Host::execute_for_ui_control`]); [`UiBridge::capability_live`]
//!   accepts it only while that exact request is pending: not finally answered, not past its deadline (compared in
//!   constant time). A reply or a timeout removes the entry, so the capability is dead from then on. This is
//!   what binds the Debug API origin (which acts as the user, without native dialogs) to a real Debug API
//!   request instead of a flag any script in the window could set. Slow work acknowledges with `pending`; that
//!   request keeps its capability for at most ten minutes, completes through a second reply, and is polled through
//!   `ui/requests` without entering the window queue.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};
use vw_core_api::{CoreError, ErrorCode};
use vw_debug_api::{RegistryError, UiRoutes};

/// Longest a client may be parked in one poll.
const MAX_POLL: Duration = Duration::from_secs(30);
/// Bounds capability leases and result retention for acknowledged work.
const JOB_LIMIT: Duration = Duration::from_secs(10 * 60);
/// Most retained results; callers should collect them promptly.
const MAX_JOBS: usize = 128;

/// A UI-control request waiting for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UiRequest {
    /// Correlation id the reply must carry.
    pub id: String,
    /// The route name without `ui/` (for example `panel`).
    pub name: String,
    /// The JSON body the caller posted.
    pub body: Value,
    /// The request's capability (module docs): passed back with each command run for this request.
    pub capability: String,
}

/// The UI's answer to one request.
#[derive(Debug, Clone, PartialEq)]
pub enum UiOutcome {
    /// The route ran; this is its result.
    Ok(Value),
    /// Work started; the UI sends its final reply later under the same id.
    Pending,
    /// The route failed; the Debug API caller gets this error.
    Err(CoreError),
}

/// Wire form of a reply: `{id, ok: true, value}` or `{id, ok: false, error: {code, message}}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiReply {
    /// The request id.
    pub id: String,
    /// Whether the route ran.
    pub ok: bool,
    /// The UI acknowledged slow work and will send a completion later.
    #[serde(default)]
    pub pending: bool,
    /// The result when `ok`.
    #[serde(default)]
    pub value: Option<Value>,
    /// The failure when not `ok`.
    #[serde(default)]
    pub error: Option<UiReplyError>,
}

/// The error part of a reply.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiReplyError {
    /// Stable error kind.
    pub code: ErrorCode,
    /// Human-readable text.
    pub message: String,
}

impl UiReply {
    /// Splits the reply into its id and outcome. A failed reply without an error body becomes an
    /// `internal` error rather than a silent success.
    #[must_use]
    pub fn into_parts(self) -> (String, UiOutcome) {
        let outcome = if self.ok && self.pending {
            UiOutcome::Pending
        } else if self.ok {
            UiOutcome::Ok(self.value.unwrap_or(Value::Null))
        } else {
            let error = self.error.unwrap_or(UiReplyError {
                code: ErrorCode::Internal,
                message: "the UI reported a failure without details".to_owned(),
            });
            UiOutcome::Err(CoreError::new(error.code, error.message))
        };
        (self.id, outcome)
    }
}

/// A request waiting for its reply: where the reply goes, its capability and when it times out.
struct Pending {
    reply: Option<oneshot::Sender<UiOutcome>>,
    capability: String,
    deadline: Instant,
}

struct Inner {
    queue: Mutex<Queue>,
    pending: Mutex<HashMap<String, Pending>>,
    jobs: Mutex<HashMap<String, (Instant, Value)>>,
    wake: Notify,
    counter: AtomicU64,
    timeout: Duration,
}

#[derive(Default)]
struct Queue {
    requests: VecDeque<UiRequest>,
    poll: u64,
}

/// Handle to the broker. Clone it freely.
#[derive(Clone)]
pub struct UiBridge {
    inner: Arc<Inner>,
}

impl UiBridge {
    /// A bridge whose requests fail with `timeout` when the UI does not answer within `timeout`.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                queue: Mutex::new(Queue::default()),
                pending: Mutex::new(HashMap::new()),
                jobs: Mutex::new(HashMap::new()),
                wake: Notify::new(),
                counter: AtomicU64::new(1),
                timeout,
            }),
        }
    }

    /// Parks a request for the UI and waits for its answer.
    ///
    /// # Errors
    /// The UI's own error, or `timeout` when no UI client answered in time.
    pub async fn request(&self, name: &str, body: Value) -> Result<Value, CoreError> {
        let id = format!("ui-{}", self.inner.counter.fetch_add(1, Ordering::Relaxed));
        let capability = new_capability()?;
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self
                .inner
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            pending.retain(|_, entry| Instant::now() < entry.deadline);
            pending.insert(
                id.clone(),
                Pending {
                    reply: Some(tx),
                    capability: capability.clone(),
                    deadline: Instant::now() + self.inner.timeout,
                },
            );
        }
        self.inner
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .push_back(UiRequest {
                id: id.clone(),
                name: name.to_owned(),
                body,
                capability,
            });
        self.inner.wake.notify_waiters();
        match tokio::time::timeout(self.inner.timeout, rx).await {
            Ok(Ok(UiOutcome::Ok(value))) => Ok(value),
            Ok(Ok(UiOutcome::Err(error))) => Err(error),
            Ok(Ok(UiOutcome::Pending)) => Ok(json!({
                "pendingId": id, "state": "pending", "poll": "/ui/requests"
            })),
            Ok(Err(_)) | Err(_) => {
                self.forget(&id);
                Err(CoreError::new(
                    ErrorCode::Timeout,
                    format!(
                        "the UI did not answer ui/{name} within {} s (is the window open?)",
                        self.inner.timeout.as_secs()
                    ),
                ))
            }
        }
    }

    fn forget(&self, id: &str) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        self.inner
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .retain(|r| r.id != id);
    }

    /// The next waiting request, parking up to `wait` (capped at 30 s) when there is none.
    /// A new poll replaces any older waiting poll, including one whose page went away while its
    /// transport stayed connected. `None` means nothing arrived or a newer poll replaced this one.
    pub async fn next_request(&self, wait: Duration) -> Option<UiRequest> {
        let wait = wait.min(MAX_POLL);
        let deadline = tokio::time::Instant::now() + wait;
        let poll = {
            let mut queue = self
                .inner
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            queue.poll = queue.poll.wrapping_add(1);
            queue.poll
        };
        self.inner.wake.notify_waiters();
        loop {
            // Register before checking the queue: a request or replacement between that check
            // and awaiting must wake this poll too.
            let wake = self.inner.wake.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            {
                let mut queue = self
                    .inner
                    .queue
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if queue.poll != poll {
                    return None;
                }
                if let Some(request) = queue.requests.pop_front() {
                    return Some(request);
                }
            }
            if tokio::time::timeout_at(deadline, wake).await.is_err() {
                return None;
            }
        }
    }

    /// Delivers the UI's answer. Returns `false` when the id is unknown or already finished (for
    /// example the request timed out meanwhile); the answer is dropped then.
    pub fn reply(&self, id: &str, outcome: UiOutcome) -> bool {
        let mut pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if matches!(outcome, UiOutcome::Pending) {
            let Some(entry) = pending.get_mut(id) else {
                return false;
            };
            if Instant::now() >= entry.deadline {
                return false;
            }
            let Some(sender) = entry.reply.take() else {
                return false;
            };
            entry.deadline = Instant::now() + JOB_LIMIT;
            let mut jobs = self
                .inner
                .jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            jobs.retain(|_, (at, _)| at.elapsed() < JOB_LIMIT);
            if jobs.len() >= MAX_JOBS
                && let Some(oldest) = jobs
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(id, _)| id.clone())
            {
                jobs.remove(&oldest);
            }
            jobs.insert(
                id.to_owned(),
                (Instant::now(), json!({"pendingId": id, "state": "pending"})),
            );
            if sender.send(UiOutcome::Pending).is_err() {
                pending.remove(id);
                jobs.remove(id);
                return false;
            }
            return true;
        }
        let Some(entry) = pending.remove(id) else {
            return false;
        };
        if let Some(sender) = entry.reply {
            return sender.send(outcome).is_ok();
        }
        if Instant::now() >= entry.deadline {
            return false;
        }
        let result = match outcome {
            UiOutcome::Ok(value) => json!({"pendingId": id, "state": "completed", "value": value}),
            UiOutcome::Err(error) => json!({"pendingId": id, "state": "failed", "error": error}),
            UiOutcome::Pending => return false,
        };
        self.inner
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned(), (Instant::now(), result));
        true
    }

    /// Reads a slow request without going through the UI queue. Results are kept for ten minutes.
    ///
    /// # Errors
    /// `notFound` for an unknown or expired id.
    pub fn request_status(&self, id: &str) -> Result<Value, CoreError> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        jobs.retain(|_, (at, _)| at.elapsed() < JOB_LIMIT);
        jobs.get(id).map(|(_, value)| value.clone()).ok_or_else(|| {
            CoreError::new(ErrorCode::NotFound, "unknown or expired pending UI request")
        })
    }

    /// True while `capability` belongs to a request that is pending: handed out or queued, not answered, not
    /// past its deadline (module docs). Every pending entry is compared in constant time, and an empty or
    /// made-up value matches none.
    #[must_use]
    pub fn capability_live(&self, capability: &str) -> bool {
        let now = Instant::now();
        let mut pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        pending.retain(|_, entry| now < entry.deadline);
        // No early return: the time does not depend on which entry matched (hence `|` and `&`, not `||`, `&&`).
        #[allow(clippy::needless_bitwise_bool)]
        pending.values().fold(false, |live, entry| {
            let same =
                vw_debug_api::constant_time_eq(capability.as_bytes(), entry.capability.as_bytes());
            live | (same & (now < entry.deadline))
        })
    }

    /// Registers `ui/<name>` so that calling it parks a request here.
    ///
    /// # Errors
    /// [`RegistryError`] for a malformed or duplicate name.
    pub fn register_route(
        &self,
        routes: &UiRoutes,
        name: &str,
        description: &str,
    ) -> Result<(), RegistryError> {
        let bridge = self.clone();
        let route = name.to_owned();
        routes.register(name, description, move |body| {
            let bridge = bridge.clone();
            let route = route.clone();
            async move { bridge.request(&route, body).await }
        })
    }

    /// Registers `ui/ui-client/poll` and `ui/ui-client/reply`, the transport a UI running in a
    /// browser (the dev UI against a real core) uses instead of Tauri commands.
    ///
    /// # Errors
    /// [`RegistryError`] when the names are already taken.
    pub fn register_client_routes(&self, routes: &UiRoutes) -> Result<(), RegistryError> {
        let bridge = self.clone();
        routes.register(
            "requests",
            "Poll slow UI work: {pendingId}. States: pending, completed (value), failed (error).",
            move |body| {
                let bridge = bridge.clone();
                async move {
                    let id = body
                        .get("pendingId")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            CoreError::new(ErrorCode::InvalidParams, "pendingId must be a string")
                        })?;
                    bridge.request_status(id)
                }
            },
        )?;
        let bridge = self.clone();
        routes.register(
            "ui-client/poll",
            "Long-poll for the next UI-control request (used by the browser dev UI). Body: {\"waitMs\": 0-30000}.",
            move |body| {
                let bridge = bridge.clone();
                async move {
                    let wait = body.get("waitMs").and_then(Value::as_u64).unwrap_or(25_000);
                    let request = bridge.next_request(Duration::from_millis(wait)).await;
                    Ok(json!({ "request": request }))
                }
            },
        )?;
        let bridge = self.clone();
        routes.register(
            "ui-client/reply",
            "Answer a UI-control request (used by the browser dev UI). Body: {id, ok, value|error}.",
            move |body| {
                let bridge = bridge.clone();
                async move {
                    let reply: UiReply = serde_json::from_value(body).map_err(|_| {
                        CoreError::new(
                            ErrorCode::InvalidCommand,
                            "not a valid reply: expected {id, ok, value|error}",
                        )
                    })?;
                    let (id, outcome) = reply.into_parts();
                    Ok(json!({ "accepted": bridge.reply(&id, outcome) }))
                }
            },
        )
    }
}

/// A fresh capability: 32 bytes from the OS RNG as 64 hex characters.
fn new_capability() -> Result<String, CoreError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| {
        CoreError::new(
            ErrorCode::Internal,
            format!("no randomness for a UI-control capability: {e}"),
        )
    })?;
    Ok(bytes.iter().fold(String::with_capacity(64), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    }))
}
