//! State shared by the connection handle and its tasks: pending requests, death, subscribers,
//! counters.
//!
//! Depends on: `error`, `message`, `handler`, `stderr_ring`. Called by: `connection`, `reader`,
//! `writer`.
//! Invariants:
//! - `pending` and `dead` live under one lock, so a request is either registered before death
//!   (and then failed by `mark_dead`) or sees the death and fails at once; none can hang.
//! - Fan-out never awaits: a full subscriber is dropped and counted, the reader moves on.
//! - Locks are std mutexes held for map operations only, never across an `.await`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::Value;
use tokio::sync::{Semaphore, mpsc, oneshot, watch};
use tracing::warn;

use crate::config::ConnectionConfig;
use crate::error::{DeathCause, DeathReport, ExitInfo, RpcError};
use crate::handler::Handlers;
use crate::message::Inbound;
use crate::stderr_ring::StderrRing;

/// A command for the writer task.
pub(crate) enum WriterCmd {
    /// Write one encoded frame.
    Frame(Vec<u8>),
    /// Close the server's stdin (graceful shutdown signal).
    Close,
}

pub(crate) type Reply = Result<Value, RpcError>;

struct Pending {
    method: String,
    tx: oneshot::Sender<Reply>,
}

#[derive(Default)]
struct State {
    pending: HashMap<i64, Pending>,
    dead: Option<DeathReport>,
    /// stdin was closed on purpose; new requests are refused, EOF is then expected.
    closing: bool,
}

pub(crate) struct Subscriber {
    tx: mpsc::Sender<Inbound>,
    lagged: Arc<AtomicBool>,
}

/// Monotonic counters exposed through [`crate::TransportStats`].
#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) notifications: AtomicU64,
    pub(crate) invalid_frames: AtomicU64,
    pub(crate) oversized_frames: AtomicU64,
    pub(crate) unknown_response_ids: AtomicU64,
    pub(crate) server_requests: AtomicU64,
    pub(crate) unhandled_server_requests: AtomicU64,
    pub(crate) dropped_subscribers: AtomicU64,
}

pub(crate) struct Shared {
    pub(crate) config: ConnectionConfig,
    pub(crate) counters: Counters,
    pub(crate) handlers: Handlers,
    /// The process's stderr ring; shared by every channel of a multiplexed process.
    pub(crate) stderr: Arc<StderrRing>,
    pub(crate) outbound: mpsc::Sender<WriterCmd>,
    /// Bounds unanswered requests (multiplexed channels only, see
    /// `ConnectionConfig::channel_max_in_flight`); `None` for a plain connection.
    pub(crate) in_flight: Option<Semaphore>,
    /// The channel id when this is one channel of a multiplexed process.
    pub(crate) channel_id: Option<u64>,
    state: Mutex<State>,
    subscribers: Mutex<Vec<Subscriber>>,
    death_tx: watch::Sender<Option<DeathReport>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

impl Shared {
    pub(crate) fn new(config: ConnectionConfig, outbound: mpsc::Sender<WriterCmd>) -> Self {
        let stderr = Arc::new(StderrRing::with_redactions(
            config.stderr,
            config.stderr_redactions.clone(),
        ));
        Self::with_parts(config, outbound, stderr, None)
    }

    /// State for one channel of a multiplexed process: its own requests, subscribers, handlers and
    /// death, the process's stderr ring, and a bound on unanswered requests.
    pub(crate) fn new_channel(
        config: ConnectionConfig,
        outbound: mpsc::Sender<WriterCmd>,
        stderr: Arc<StderrRing>,
        channel_id: u64,
    ) -> Self {
        Self::with_parts(config, outbound, stderr, Some(channel_id))
    }

    fn with_parts(
        config: ConnectionConfig,
        outbound: mpsc::Sender<WriterCmd>,
        stderr: Arc<StderrRing>,
        channel_id: Option<u64>,
    ) -> Self {
        let in_flight = channel_id.map(|_| Semaphore::new(config.channel_max_in_flight.max(1)));
        Self {
            config,
            counters: Counters::default(),
            handlers: Handlers::default(),
            stderr,
            outbound,
            in_flight,
            channel_id,
            state: Mutex::new(State::default()),
            subscribers: Mutex::new(Vec::new()),
            death_tx: watch::Sender::new(None),
        }
    }

    /// Registers a pending request, or returns why it cannot be sent.
    pub(crate) fn register(
        &self,
        id: i64,
        method: &str,
    ) -> Result<oneshot::Receiver<Reply>, RpcError> {
        let mut state = locked(&self.state);
        if let Some(report) = &state.dead {
            return Err(RpcError::ServerDead(Box::new(report.clone())));
        }
        if state.closing {
            return Err(RpcError::ServerDead(Box::new(
                self.report(DeathCause::ClosedByClient, None),
            )));
        }
        let (tx, rx) = oneshot::channel();
        state.pending.insert(
            id,
            Pending {
                method: method.to_owned(),
                tx,
            },
        );
        Ok(rx)
    }

    /// Forgets a pending request (timeout or caller dropped the future). Returns true if it was
    /// still pending.
    pub(crate) fn forget(&self, id: i64) -> bool {
        locked(&self.state).pending.remove(&id).is_some()
    }

    /// Routes a response to its waiter; unknown ids are counted and dropped.
    pub(crate) fn complete(&self, id: &Value, outcome: Result<Value, Value>) {
        let pending = id
            .as_i64()
            .and_then(|id| locked(&self.state).pending.remove(&id));
        let Some(Pending { method, tx }) = pending else {
            bump(&self.counters.unknown_response_ids);
            // Only the id's kind is logged: a bogus id could be an arbitrary string from the
            // server, and frame contents never go into the log stream.
            warn!(
                id = id.as_i64(),
                id_is_integer = id.is_i64(),
                "response for an unknown or abandoned request id"
            );
            return;
        };
        let reply = outcome.map_err(|raw| RpcError::Remote {
            method,
            error: crate::message::protocol_error(raw),
        });
        // The waiter may have given up in the same instant; nothing to do then.
        let _ = tx.send(reply);
    }

    /// Marks stdin as deliberately closed.
    pub(crate) fn set_closing(&self) {
        locked(&self.state).closing = true;
    }

    pub(crate) fn is_closing(&self) -> bool {
        locked(&self.state).closing
    }

    fn report(&self, cause: DeathCause, exit: Option<ExitInfo>) -> DeathReport {
        DeathReport {
            cause,
            exit,
            stderr_tail: self.stderr.tail(self.config.death_stderr_lines),
        }
    }

    /// Declares the connection dead (first cause wins), fails every waiter, ends every
    /// subscription and publishes the report. Returns the report in force.
    pub(crate) fn mark_dead(&self, cause: DeathCause, exit: Option<ExitInfo>) -> DeathReport {
        let (report, waiters) = {
            let mut state = locked(&self.state);
            if let Some(report) = &state.dead {
                return report.clone();
            }
            let report = self.report(cause, exit);
            state.dead = Some(report.clone());
            (report, std::mem::take(&mut state.pending))
        };
        // Log facts only: the stderr tail may hold prompts, file contents or credentials and
        // stays in memory, reachable through the typed report the caller asks for.
        warn!(
            cause = report.cause.kind(),
            exit_code = report.exit.and_then(|e| e.code),
            signal = report.exit.and_then(|e| e.signal),
            stderr_lines_kept = report.stderr_tail.len(),
            "server connection dead"
        );
        for (_, pending) in waiters {
            let _ = pending
                .tx
                .send(Err(RpcError::ServerDead(Box::new(report.clone()))));
        }
        locked(&self.subscribers).clear();
        self.death_tx.send_replace(Some(report.clone()));
        report
    }

    pub(crate) fn death(&self) -> Option<DeathReport> {
        locked(&self.state).dead.clone()
    }

    pub(crate) fn death_watch(&self) -> watch::Receiver<Option<DeathReport>> {
        self.death_tx.subscribe()
    }

    /// Adds a subscriber. If the connection is already dead the returned receiver ends at once.
    pub(crate) fn subscribe(&self) -> (mpsc::Receiver<Inbound>, Arc<AtomicBool>) {
        let (tx, rx) = mpsc::channel(self.config.subscriber_capacity.max(1));
        let lagged = Arc::new(AtomicBool::new(false));
        // Check death while holding the subscriber lock: `mark_dead` publishes death before it
        // clears the list, so a subscriber pushed here is either removed by that clear or never
        // pushed. Checking outside the lock let one slip in after the clear and never end.
        let mut subscribers = locked(&self.subscribers);
        if self.death().is_none() {
            subscribers.push(Subscriber {
                tx,
                lagged: lagged.clone(),
            });
        }
        drop(subscribers);
        (rx, lagged)
    }

    /// Delivers one item to every subscriber without waiting. A full subscriber is removed and
    /// flagged as lagged; a closed one is removed silently.
    pub(crate) fn fan_out(&self, item: &Inbound) {
        let mut subscribers = locked(&self.subscribers);
        subscribers.retain(|subscriber| match subscriber.tx.try_send(item.clone()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                subscriber.lagged.store(true, Ordering::Relaxed);
                bump(&self.counters.dropped_subscribers);
                warn!("dropping a subscriber that fell behind");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        });
    }
}
