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
//! - The bridge carries only what the route and the UI put in it; it never reaches Vibe, so it
//!   cannot be a path around the gate. A route that needs Vibe must go through `Core::execute`.
//! - **Capability:** every request carries a fresh random `capability` (256 bits
//!   from the OS RNG) that only the UI client taking the request sees. While the UI serves it, the commands it
//!   runs for it pass that capability ([`crate::Host::execute_for_ui_control`]); [`UiBridge::capability_live`]
//!   accepts it only while that exact request is pending: not yet answered, not past its deadline (compared in
//!   constant time). A reply or a timeout removes the entry, so the capability is dead from then on. This is
//!   what binds the Debug API origin (which acts as the user, without native dialogs) to a real Debug API
//!   request instead of a flag any script in the window could set.

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
        let outcome = if self.ok {
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
    reply: oneshot::Sender<UiOutcome>,
    capability: String,
    deadline: Instant,
}

struct Inner {
    queue: Mutex<VecDeque<UiRequest>>,
    pending: Mutex<HashMap<String, Pending>>,
    wake: Notify,
    counter: AtomicU64,
    timeout: Duration,
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
                queue: Mutex::new(VecDeque::new()),
                pending: Mutex::new(HashMap::new()),
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
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id.clone(),
                Pending {
                    reply: tx,
                    capability: capability.clone(),
                    deadline: Instant::now() + self.inner.timeout,
                },
            );
        self.inner
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(UiRequest {
                id: id.clone(),
                name: name.to_owned(),
                body,
                capability,
            });
        self.inner.wake.notify_one();
        match tokio::time::timeout(self.inner.timeout, rx).await {
            Ok(Ok(UiOutcome::Ok(value))) => Ok(value),
            Ok(Ok(UiOutcome::Err(error))) => Err(error),
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
            .retain(|r| r.id != id);
    }

    /// The next waiting request, parking up to `wait` (capped at 30 s) when there is none.
    /// `None` means nothing arrived; the client polls again.
    pub async fn next_request(&self, wait: Duration) -> Option<UiRequest> {
        let wait = wait.min(MAX_POLL);
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(request) = self
                .inner
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
            {
                return Some(request);
            }
            if tokio::time::timeout_at(deadline, self.inner.wake.notified())
                .await
                .is_err()
            {
                return None;
            }
        }
    }

    /// Delivers the UI's answer. Returns `false` when the id is unknown or already finished (for
    /// example the request timed out meanwhile); the answer is dropped then.
    pub fn reply(&self, id: &str, outcome: UiOutcome) -> bool {
        let sender = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        sender.is_some_and(|pending| pending.reply.send(outcome).is_ok())
    }

    /// True while `capability` belongs to a request that is pending: handed out or queued, not answered, not
    /// past its deadline (module docs). Every pending entry is compared in constant time, and an empty or
    /// made-up value matches none.
    #[must_use]
    pub fn capability_live(&self, capability: &str) -> bool {
        let now = Instant::now();
        let pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
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
