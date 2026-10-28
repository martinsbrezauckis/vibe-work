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
//! - **No authority reaches the page.** A request handed to the UI carries only its id, route and body. The page's
//!   JavaScript holds nothing that makes its commands the Debug API's: every window command runs as the window
//!   (`Origin::Ui`, [`crate::Host::execute_window`]), whoever sent it, so a script injected into the page gains
//!   nothing by hooking the poll, the replies or the outgoing calls.
//! - **Grants, decided here.** The few owner-only effects a route promises (a folder given as a path, a push whose
//!   native confirmation the Debug API answers, ...) are computed in Rust from the authenticated request's route and
//!   body when it is parked ([`crate::ui_grants`]): each is a pattern over one exact window command, pinned to the
//!   values the request names. While the request is pending, the first window command that matches a grant runs
//!   as the Debug API, once ([`UiBridge::take_grant`]); a reply, a timeout or turning the Debug API off ends every
//!   grant of that request. A page cannot widen, renew or create a grant: nothing it sends is read to build one.
//! - **Driving.** While a request that may act is pending (any but a pure read, [`crate::ui_grants::drives`]) the
//!   window is driven by the Debug API ([`UiBridge::driving`]): the core attributes window commands to the Debug API
//!   and holds them to its vault rule. That only restricts; it never grants.
//! - Slow work acknowledges with `pending`; that request (and its unused grants) lasts at most ten minutes,
//!   completes through a second reply, and is polled through `ui/requests` without entering the window queue.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};
use vw_core_api::{CoreError, ErrorCode};
use vw_debug_api::{RegistryError, UiRoutes};

use crate::ui_grants::{Grant, GrantFacts};

/// Longest a client may be parked in one poll.
const MAX_POLL: Duration = Duration::from_secs(30);
/// Bounds acknowledged work (its grants included) and result retention.
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
    /// Optional machine-readable refusal reason from the core, such as `vaultUserOnly`.
    #[serde(default)]
    pub server_code: Option<String>,
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
                server_code: None,
            });
            let mut core_error = CoreError::new(error.code, error.message);
            core_error.server_code = error.server_code;
            UiOutcome::Err(core_error)
        };
        (self.id, outcome)
    }
}

/// A request waiting for its reply: where the reply goes, when it times out and the window commands it lets run
/// as the Debug API (module docs).
struct Pending {
    reply: Option<oneshot::Sender<UiOutcome>>,
    deadline: Instant,
    grants: Vec<Grant>,
    /// May act (not a pure read): counts for [`UiBridge::driving`].
    drives: bool,
    /// Screenshot checks need geometry, never a slow-work acknowledgement.
    final_only: bool,
}

struct Inner {
    queue: Mutex<Queue>,
    pending: Mutex<HashMap<String, Pending>>,
    jobs: Mutex<HashMap<String, (Instant, Value)>>,
    wake: Notify,
    counter: AtomicU64,
    timeout: Duration,
    /// Host-side facts grants may pin (the core's catalog), set once the core runs.
    facts: std::sync::OnceLock<Arc<dyn GrantFacts>>,
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
                facts: std::sync::OnceLock::new(),
            }),
        }
    }

    /// Gives grants the host's facts (the core's catalog); the first call wins.
    pub fn set_facts(&self, facts: Arc<dyn GrantFacts>) {
        let _ = self.inner.facts.set(facts);
    }

    /// Parks a request for the UI and waits for its answer. Its grants are built here, from `name` and `body`
    /// only ([`crate::ui_grants`]).
    ///
    /// # Errors
    /// The UI's own error, or `timeout` when no UI client answered in time.
    pub async fn request(&self, name: &str, body: Value) -> Result<Value, CoreError> {
        let id = format!("ui-{}", self.inner.counter.fetch_add(1, Ordering::Relaxed));
        // From the authenticated request alone, before the page sees it.
        let grants =
            crate::ui_grants::for_request(name, &body, self.inner.facts.get().map(AsRef::as_ref));
        let drives = crate::ui_grants::drives(name, &body);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self
                .inner
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            Self::prune(&mut pending);
            pending.insert(
                id.clone(),
                Pending {
                    reply: Some(tx),
                    deadline: Instant::now() + self.inner.timeout,
                    grants,
                    drives,
                    final_only: name == "secrets",
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
            if entry.final_only {
                // Keep the original waiter and deadline: pixels must not leave before the final check.
                return entry.reply.is_some();
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
        // The request is over: its unused grants went with the entry.
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

    /// Drops entries past their deadline (a timed-out request whose waiter is gone), ending their grants.
    fn prune(pending: &mut HashMap<String, Pending>) {
        let now = Instant::now();
        pending.retain(|_, entry| now < entry.deadline);
    }

    /// True while the Debug API drives the window: a request that may act (any but a pure read) is pending
    /// (queued, being served, or acknowledged and not finished). The core attributes window commands to the Debug API
    /// then and holds them to its vault rule (module docs); nothing is granted by it.
    #[must_use]
    pub fn driving(&self) -> bool {
        let mut pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Self::prune(&mut pending);
        pending.values().any(|entry| entry.drives)
    }

    /// Uses the first unused grant of a pending request that `command` (a window command as JSON) matches, and
    /// returns that request's id and the grant's name; `None` when no grant matches (the command runs as the window).
    /// A grant is used once; a second identical command runs as the window.
    #[must_use]
    pub fn take_grant(&self, command: &Value) -> Option<(String, &'static str)> {
        let mut pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Self::prune(&mut pending);
        // Oldest request first: ids are `ui-<counter>`, so sort by the counter for a stable choice.
        let mut ids: Vec<&String> = pending.keys().collect();
        ids.sort_by_key(|id| {
            id.trim_start_matches("ui-")
                .parse::<u64>()
                .unwrap_or(u64::MAX)
        });
        let id = ids
            .into_iter()
            .find(|id| pending[*id].grants.iter().any(|g| g.matches(command)))?
            .clone();
        let entry = pending.get_mut(&id)?;
        let grant = entry.grants.iter_mut().find(|g| g.matches(command))?;
        grant.used = true;
        Some((id, grant.name))
    }

    /// Ends every unused grant (the Debug API was turned off): pending requests may still be answered, but their
    /// commands run as the window's from now on.
    pub fn revoke_grants(&self) {
        for entry in self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values_mut()
        {
            entry.grants.clear();
        }
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
