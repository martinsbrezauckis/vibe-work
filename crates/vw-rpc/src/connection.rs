//! The public connection handle: requests, notifications, subscriptions, server-request
//! handlers, death and statistics.
//!
//! Depends on: every other module of this crate; tokio. Called by: vw-supervisor (one connection
//! per server process), and later vw-session / vw-gate through the supervisor.
//! Invariants:
//! - Every request ends: a result, a remote error, a timeout, or `ServerDead`; never a hang.
//! - Dropping a request future cancels it (the pending entry is removed; a late answer is counted
//!   as an unknown id and dropped).
//! - The connection's tasks are owned. An opted-in multiplex channel transfers its writer to
//!   bounded interrupt cleanup on last-handle Drop; other connections abort their tasks on Drop.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::debug;
use vw_protocol::RequestMethod;

use crate::config::{ConnectionConfig, RequestOptions, TimeoutPolicy};
use crate::error::{DeathCause, DeathReport, ExitInfo, RpcError};
use crate::framing::encode_frame;
use crate::handler::ServerRequestHandler;
use crate::message::{Inbound, next_request_id, notification_frame, request_frame};
use crate::reader::{self, EndSignals};
use crate::shared::{Shared, WriterCmd};
use crate::stderr_ring;

/// Snapshot of a connection's counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransportStats {
    /// Notifications received.
    pub notifications: u64,
    /// Lines that were not valid JSON-RPC objects (discarded).
    pub invalid_frames: u64,
    /// Frames over the size limit (discarded).
    pub oversized_frames: u64,
    /// Responses whose id matched no pending request (late, duplicate or bogus).
    pub unknown_response_ids: u64,
    /// Server requests received.
    pub server_requests: u64,
    /// Server requests answered with `method_not_found` because no handler was registered.
    pub unhandled_server_requests: u64,
    /// Subscribers dropped for falling behind.
    pub dropped_subscribers: u64,
    /// stderr lines seen.
    pub stderr_lines: u64,
}

type DropCleanup = Box<dyn FnOnce(Vec<JoinHandle<()>>, Option<Arc<Tasks>>) + Send + Sync>;

/// Owns the connection's tasks and optional close cleanup. Multiplexed channels retain the process
/// tasks (`parent`) until their last handle and any bounded interrupt cleanup finish.
pub(crate) struct Tasks {
    own: Vec<JoinHandle<()>>,
    parent: Option<Arc<Tasks>>,
    /// Runs its `Drop` after the tasks are aborted (a channel uses it to close itself on the server).
    guard: Option<Box<dyn Send + Sync>>,
    cleanup: Option<DropCleanup>,
}

impl Tasks {
    /// Owns `own`; with `parent`, also keeps the process's tasks alive.
    pub(crate) fn new(own: Vec<JoinHandle<()>>, parent: Option<Arc<Tasks>>) -> Self {
        Self {
            own,
            parent,
            guard: None,
            cleanup: None,
        }
    }

    /// Adds a value dropped right after the tasks are aborted.
    pub(crate) fn with_guard(mut self, guard: impl Send + Sync + 'static) -> Self {
        self.guard = Some(Box::new(guard));
        self
    }

    pub(crate) fn with_cleanup(
        mut self,
        cleanup: impl FnOnce(Vec<JoinHandle<()>>, Option<Arc<Tasks>>) + Send + Sync + 'static,
    ) -> Self {
        self.cleanup = Some(Box::new(cleanup));
        self
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup(std::mem::take(&mut self.own), self.parent.take());
            return;
        }
        for task in &self.own {
            task.abort();
        }
        drop(self.guard.take());
        // `parent` drops last: the process outlives the abort of the channel's own tasks.
        drop(self.parent.take());
    }
}

/// Builder for a [`Connection`]. Register handlers here so none of the server's first requests
/// can arrive before its handler exists.
pub struct ConnectionBuilder {
    config: ConnectionConfig,
    handlers: Vec<(String, Arc<dyn ServerRequestHandler>)>,
    stderr: Option<Box<dyn AsyncRead + Send + Unpin>>,
    exit: Option<watch::Receiver<Option<ExitInfo>>>,
}

impl ConnectionBuilder {
    /// A builder with the given tuning.
    #[must_use]
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            handlers: Vec::new(),
            stderr: None,
            exit: None,
        }
    }

    /// Registers the handler for one server-request method.
    #[must_use]
    pub fn handler(
        mut self,
        method: impl Into<String>,
        handler: impl ServerRequestHandler,
    ) -> Self {
        self.handlers.push((method.into(), Arc::new(handler)));
        self
    }

    /// Registers a shared handler (the supervisor re-registers the same handlers on every new
    /// connection after a restart).
    #[must_use]
    pub fn handler_arc(
        mut self,
        method: impl Into<String>,
        handler: Arc<dyn ServerRequestHandler>,
    ) -> Self {
        self.handlers.push((method.into(), handler));
        self
    }

    /// Captures the server's stderr into the bounded ring (diagnostics only, never protocol).
    #[must_use]
    pub fn stderr(mut self, stderr: impl AsyncRead + Send + Unpin + 'static) -> Self {
        self.stderr = Some(Box::new(stderr));
        self
    }

    /// Supplies the server's exit status, so death reports after EOF carry it.
    #[must_use]
    pub fn exit_watch(mut self, exit: watch::Receiver<Option<ExitInfo>>) -> Self {
        self.exit = Some(exit);
        self
    }

    /// Starts the reader, writer and stderr tasks and returns the handle.
    ///
    /// `stdout` is the server's output (our input), `stdin` the server's input.
    ///
    /// # Panics
    /// Never by itself; like any `tokio::spawn`, it must be called inside a tokio runtime.
    pub fn start<R, W>(self, stdout: R, stdin: W) -> Connection
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (out_tx, out_rx) = mpsc::channel(self.config.outbound_queue.max(1));
        let shared = Arc::new(Shared::new(self.config, out_tx));
        for (method, handler) in self.handlers {
            shared.handlers.insert(method, handler);
        }
        let mut tasks = Vec::with_capacity(3);
        let mut stderr_done = None;
        if let Some(stderr) = self.stderr {
            let (done_tx, done_rx) = watch::channel(false);
            stderr_done = Some(done_rx);
            let pump_shared = shared.clone();
            tasks.push(tokio::spawn(async move {
                stderr_ring::pump(stderr, &pump_shared.stderr).await;
                done_tx.send_replace(true);
            }));
        }
        tasks.push(tokio::spawn(crate::writer::run(
            stdin,
            out_rx,
            shared.clone(),
        )));
        let end = EndSignals {
            exit: self.exit,
            stderr_done,
        };
        tasks.push(tokio::spawn(reader::run(stdout, shared.clone(), end)));
        Connection {
            shared,
            tasks: Arc::new(Tasks::new(tasks, None)),
        }
    }
}

/// A JSON-RPC 2.0 connection to one server over NDJSON streams. Cheap to clone.
#[derive(Clone)]
pub struct Connection {
    pub(crate) shared: Arc<Shared>,
    tasks: Arc<Tasks>,
}

/// A connection reference that does not keep the connection alive (for handlers).
#[derive(Clone)]
pub struct WeakConnection {
    shared: std::sync::Weak<Shared>,
    tasks: std::sync::Weak<Tasks>,
}

impl WeakConnection {
    /// The connection, if it still exists.
    #[must_use]
    pub fn upgrade(&self) -> Option<Connection> {
        Some(Connection {
            shared: self.shared.upgrade()?,
            tasks: self.tasks.upgrade()?,
        })
    }
}

/// Removes the pending entry if the request future is dropped before an answer.
struct PendingGuard<'a> {
    shared: &'a Shared,
    id: i64,
    armed: bool,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if self.armed && self.shared.forget(self.id) {
            debug!(id = self.id, "request cancelled by the caller");
        }
    }
}

impl Connection {
    /// A handle over existing state and tasks (one channel of a multiplexed process, `mux.rs`).
    pub(crate) fn from_parts(shared: Arc<Shared>, tasks: Arc<Tasks>) -> Self {
        shared.set_owner(&tasks);
        Self { shared, tasks }
    }

    /// The channel id when this connection is one channel of a multiplexed process
    /// (`vibe-app-server --multiplex`); `None` for a plain stdio connection.
    #[must_use]
    pub fn channel_id(&self) -> Option<u64> {
        self.shared.channel_id
    }

    /// Sends a request and waits for its answer with the connection's default timeout.
    ///
    /// # Errors
    /// [`RpcError::Remote`] for an error answer, [`RpcError::Timeout`] when no answer came in
    /// time, [`RpcError::ServerDead`] when the server is or becomes dead (including a timeout on
    /// a wedge method such as `callback/result`), [`RpcError::Encode`] never for a `Value`.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.request_with(method, params, RequestOptions::default())
            .await
    }

    /// [`Connection::request`] with per-request options.
    ///
    /// # Errors
    /// As [`Connection::request`].
    pub async fn request_with(
        &self,
        method: &str,
        params: Value,
        options: RequestOptions,
    ) -> Result<Value, RpcError> {
        self.request_inner(method, params, options, false).await
    }

    pub(crate) async fn close_request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.request_inner(method, params, RequestOptions::default(), true)
            .await
    }

    async fn request_inner(
        &self,
        method: &str,
        params: Value,
        options: RequestOptions,
        closing: bool,
    ) -> Result<Value, RpcError> {
        let shared = &*self.shared;
        let id = next_request_id();
        let after = options.timeout.unwrap_or_else(|| {
            shared
                .config
                .method_timeouts
                .iter()
                .find(|(m, _)| m == method)
                .map_or(shared.config.default_timeout, |(_, t)| *t)
        });
        let policy = options.on_timeout.unwrap_or_else(|| {
            if shared.config.wedge_methods.iter().any(|m| m == method) {
                TimeoutPolicy::ServerDead
            } else {
                TimeoutPolicy::Fail
            }
        });
        // Stop sends these requests before it reaches channel cleanup. An unanswered Stop
        // must end the server within the same budget, even if its caller supplied a longer
        // timeout or a nonfatal policy; otherwise that early error would skip close entirely.
        let (after, policy) = if shared.config.interrupt_before_close
            && matches!(method, "turn/interrupt" | "session/subagents/interrupt")
        {
            (
                after.min(shared.config.close_timeout),
                TimeoutPolicy::ServerDead,
            )
        } else {
            (after, policy)
        };
        let bytes = encode_frame(&request_frame(id, method, &params)).map_err(|source| {
            RpcError::Encode {
                method: method.to_owned(),
                source,
            }
        })?;
        let rx = shared.register(id, method, &params, closing)?;
        let mut guard = PendingGuard {
            shared,
            id,
            armed: true,
        };
        debug!(id, method, "request");
        let exchange = async {
            // A channel bounds its unanswered requests (`channel_max_in_flight`); the permit is held
            // until the answer arrives or the request ends otherwise.
            let _permit = match &shared.in_flight {
                Some(slots) => match slots.acquire().await {
                    Ok(permit) => Some(permit),
                    Err(_) => return Err(self.dead_error()),
                },
                None => None,
            };
            if shared.outbound.send(WriterCmd::Frame(bytes)).await.is_err() {
                return Err(self.dead_error());
            }
            rx.await.unwrap_or_else(|_| Err(self.dead_error()))
        };
        let outcome = tokio::time::timeout(after, exchange).await;
        guard.armed = false;
        let Ok(reply) = outcome else {
            shared.forget(id);
            if policy == TimeoutPolicy::ServerDead {
                let cause = DeathCause::Wedged {
                    method: method.to_owned(),
                    id,
                    after,
                };
                return Err(RpcError::ServerDead(Box::new(
                    shared.mark_dead(cause, None),
                )));
            }
            return Err(RpcError::Timeout {
                method: method.to_owned(),
                id,
                after,
            });
        };
        if method == "session/stop" && reply.is_ok() && shared.stop_acknowledged() {
            // Its owned receipt watchdog survives cancellation and bounds this wait. Resume
            // must not reuse the old channel or race the release of its session lease.
            let report = self.closed().await;
            if report.cause != DeathCause::ClosedByClient {
                return Err(RpcError::ServerDead(Box::new(report)));
            }
        }
        reply
    }

    /// Typed request: params and result types come from the generated method registry.
    ///
    /// # Errors
    /// As [`Connection::request`], plus [`RpcError::Encode`] / [`RpcError::Decode`] when the
    /// params or the answer do not match the generated types.
    pub async fn call<M>(&self, params: &M::Params) -> Result<M::Result, RpcError>
    where
        M: RequestMethod,
        M::Params: Serialize,
        M::Result: DeserializeOwned,
    {
        let params = serde_json::to_value(params).map_err(|source| RpcError::Encode {
            method: M::NAME.to_owned(),
            source,
        })?;
        let result = self.request(M::NAME, params).await?;
        serde_json::from_value(result).map_err(|source| RpcError::Decode {
            method: M::NAME.to_owned(),
            source,
        })
    }

    /// Sends a notification (no answer).
    ///
    /// # Errors
    /// [`RpcError::ServerDead`] when the connection is dead or closing.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), RpcError> {
        if self.shared.death().is_some() || self.shared.is_closing() {
            return Err(self.dead_error());
        }
        let bytes = encode_frame(&notification_frame(method, &params)).map_err(|source| {
            RpcError::Encode {
                method: method.to_owned(),
                source,
            }
        })?;
        self.shared
            .outbound
            .send(WriterCmd::Frame(bytes))
            .await
            .map_err(|_| self.dead_error())
    }

    /// Subscribes to notifications and gaps from now on.
    #[must_use]
    pub fn subscribe(&self) -> Subscription {
        let (rx, lagged) = self.shared.subscribe();
        Subscription { rx, lagged }
    }

    /// The death report, if the server is dead.
    #[must_use]
    pub fn death(&self) -> Option<DeathReport> {
        self.shared.death()
    }

    /// A watch that becomes `Some(report)` when the server dies.
    #[must_use]
    pub fn death_watch(&self) -> watch::Receiver<Option<DeathReport>> {
        self.shared.death_watch()
    }

    /// Waits until the server is dead and returns the report.
    pub async fn closed(&self) -> DeathReport {
        let mut watch = self.shared.death_watch();
        loop {
            if let Some(report) = watch.borrow_and_update().clone() {
                return report;
            }
            if watch.changed().await.is_err() {
                // Sender lives in `Shared`, which we hold: unreachable, but never hang.
                return self.shared.mark_dead(DeathCause::ClosedByClient, None);
            }
        }
    }

    /// Fences new requests and closes input. With `interrupt_before_close`, an owned bounded task
    /// stops attached work first; an unverified stop instead marks the process dead for its owner.
    /// Answers already on their way still arrive. Cancelling this caller does not cancel cleanup.
    pub async fn close_input(&self) {
        if self.shared.config.interrupt_before_close {
            self.start_close();
            self.wait_close_sent().await;
            return;
        }
        self.shared.set_closing();
        let _ = self.shared.outbound.send(WriterCmd::Close).await;
    }

    /// Counter snapshot.
    #[must_use]
    pub fn stats(&self) -> TransportStats {
        let c = &self.shared.counters;
        let get = |counter: &std::sync::atomic::AtomicU64| counter.load(Ordering::Relaxed);
        TransportStats {
            notifications: get(&c.notifications),
            invalid_frames: get(&c.invalid_frames),
            oversized_frames: get(&c.oversized_frames),
            unknown_response_ids: get(&c.unknown_response_ids),
            server_requests: get(&c.server_requests),
            unhandled_server_requests: get(&c.unhandled_server_requests),
            dropped_subscribers: get(&c.dropped_subscribers),
            stderr_lines: self.shared.stderr.total_lines(),
        }
    }

    /// The last `n` stderr lines (oldest first).
    #[must_use]
    pub fn stderr_tail(&self, n: usize) -> Vec<String> {
        self.shared.stderr.tail(n)
    }

    /// A non-owning reference for handlers.
    #[must_use]
    pub fn downgrade(&self) -> WeakConnection {
        WeakConnection {
            shared: Arc::downgrade(&self.shared),
            tasks: Arc::downgrade(&self.tasks),
        }
    }

    fn dead_error(&self) -> RpcError {
        let report = self.shared.death().unwrap_or_else(|| DeathReport {
            cause: DeathCause::ClosedByClient,
            exit: None,
            stderr_tail: self
                .shared
                .stderr
                .tail(self.shared.config.death_stderr_lines),
        });
        RpcError::ServerDead(Box::new(report))
    }
}

/// A stream of [`Inbound`] items for one subscriber.
pub struct Subscription {
    rx: mpsc::Receiver<Inbound>,
    lagged: Arc<AtomicBool>,
}

impl Subscription {
    /// The next item; `None` when the connection is dead or this subscriber was dropped for
    /// falling behind (check [`Subscription::was_dropped_for_lag`]).
    pub async fn recv(&mut self) -> Option<Inbound> {
        self.rx.recv().await
    }

    /// True when the connection dropped this subscriber because its buffer was full. Its view
    /// is stale: resubscribe and re-read state.
    #[must_use]
    pub fn was_dropped_for_lag(&self) -> bool {
        self.lagged.load(Ordering::Relaxed)
    }
}
