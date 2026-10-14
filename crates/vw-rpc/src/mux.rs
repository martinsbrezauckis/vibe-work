//! Multiplexed stdio: one `vibe-app-server --multiplex` process carries many logical connections
//! ("channels"); each channel is a full [`Connection`] with its own request ids, answers, server
//! requests, subscribers and death.
//!
//! Wire format (Vibe 2.26.0 `app_server/multiplex.py`, read): every line both ways is
//! `{"channelId": "<n>", "message": <JSON-RPC object> | null}` with `channelId` matching
//! `^[1-9][0-9]{0,15}$`. A frame for an unknown id opens that channel on the server, but only when the id
//! is above every id opened before in this process (lower ids are ignored silently). `message: null`
//! from us closes a channel; the server answers `{"channelId", "message": null}` (plus `"error"` when
//! the channel's server failed) after its harness flushed and released the session lease. The server
//! validates the outer frame strictly (`extra="forbid"`) and a bad one ends the WHOLE process, so this
//! module only ever writes the two keys, a decimal id string and an object or `null`.
//!
//! Depends on: `connection` (the per-channel handle), `shared`, `reader` (`dispatch_value`), `writer`,
//! `framing`, `stderr_ring`. Called by: vw-supervisor (one [`Multiplexer`] per server process in
//! multiplex mode), tests.
//! Invariants:
//! - Channel ids are allocated by this process-side handle only, strictly increasing from 1, never reused
//!   (a new process starts again at 1).
//! - The process reader parses each line once and never awaits a channel: inner messages go through
//!   the same non-blocking dispatch as a plain connection (a slow subscriber of one channel is dropped
//!   as lagged; no other channel waits for it).
//! - Outbound frames are wrapped by byte splicing around the already encoded inner frame (no second
//!   serialization of large messages).
//! - Process death (stdout EOF, read/write failure) kills every channel with the same cause.
//! - A channel whose last handle is dropped is closed on the server (best effort, non-blocking), so a
//!   forgotten channel never keeps a session lease.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::config::ConnectionConfig;
use crate::connection::{Connection, Tasks};
use crate::error::{DeathCause, DeathReport, ExitInfo, RpcError};
use crate::framing::{BoundedLine, read_bounded_line};
use crate::handler::ServerRequestHandler;
use crate::message::{GapReason, Inbound};
use crate::reader::{self, EndSignals};
use crate::shared::{Shared, WriterCmd, bump};
use crate::stderr_ring;

/// Largest channel id the server accepts (`^[1-9][0-9]{0,15}$`: at most 16 digits).
pub const MAX_CHANNEL_ID: u64 = 9_999_999_999_999_999;

/// Bytes the outer frame adds around an inner message (key names, quotes, a 16-digit id).
const WRAPPER_BYTES: usize = 64;

/// Counters of one multiplexed process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MuxStats {
    /// Channels opened so far (equals the last id allocated).
    pub channels_opened: u64,
    /// Channels open right now (closed ones leave when the server acknowledges the close).
    pub open_channels: usize,
    /// Server frames for a channel id this side does not have open (dropped).
    pub unknown_channel_frames: u64,
    /// Lines that were not a valid channel frame (reported to every channel as a gap).
    pub invalid_frames: u64,
    /// Lines over the frame limit (reported to every channel as a gap).
    pub oversized_frames: u64,
}

/// Builder for a [`Multiplexer`]. Handlers registered here are installed on every channel before
/// the channel's first frame is sent.
pub struct MultiplexerBuilder {
    config: ConnectionConfig,
    handlers: Vec<(String, Arc<dyn ServerRequestHandler>)>,
    stderr: Option<Box<dyn AsyncRead + Send + Unpin>>,
    exit: Option<watch::Receiver<Option<ExitInfo>>>,
}

impl MultiplexerBuilder {
    /// A builder with the given tuning (applies to the process and to every channel).
    #[must_use]
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            handlers: Vec::new(),
            stderr: None,
            exit: None,
        }
    }

    /// Registers a shared server-request handler for every channel.
    #[must_use]
    pub fn handler_arc(
        mut self,
        method: impl Into<String>,
        handler: Arc<dyn ServerRequestHandler>,
    ) -> Self {
        self.handlers.push((method.into(), handler));
        self
    }

    /// Registers a server-request handler for every channel.
    #[must_use]
    pub fn handler(
        mut self,
        method: impl Into<String>,
        handler: impl ServerRequestHandler,
    ) -> Self {
        self.handlers.push((method.into(), Arc::new(handler)));
        self
    }

    /// Captures the process's stderr into one bounded ring that every channel reports from.
    #[must_use]
    pub fn stderr(mut self, stderr: impl AsyncRead + Send + Unpin + 'static) -> Self {
        self.stderr = Some(Box::new(stderr));
        self
    }

    /// Supplies the process's exit status, so death reports after EOF carry it.
    #[must_use]
    pub fn exit_watch(mut self, exit: watch::Receiver<Option<ExitInfo>>) -> Self {
        self.exit = Some(exit);
        self
    }

    /// Starts the process reader, the stdin writer and the stderr pump. No channel is open yet.
    ///
    /// # Panics
    /// Never by itself; like any `tokio::spawn`, it must be called inside a tokio runtime.
    pub fn start<R, W>(self, stdout: R, stdin: W) -> Multiplexer
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (out_tx, out_rx) = mpsc::channel(self.config.outbound_queue.max(1));
        let process = Arc::new(Shared::new(self.config.clone(), out_tx));
        let mux = Arc::new(MuxShared {
            config: self.config,
            process: process.clone(),
            handlers: self.handlers,
            channels: Mutex::new(HashMap::new()),
            last_id: AtomicU64::new(0),
            unknown_channel_frames: AtomicU64::new(0),
        });
        let mut tasks = Vec::with_capacity(3);
        let mut stderr_done = None;
        if let Some(stderr) = self.stderr {
            let (done_tx, done_rx) = watch::channel(false);
            stderr_done = Some(done_rx);
            let pump = process.clone();
            tasks.push(tokio::spawn(async move {
                stderr_ring::pump(stderr, &pump.stderr).await;
                done_tx.send_replace(true);
            }));
        }
        tasks.push(tokio::spawn(crate::writer::run(
            stdin,
            out_rx,
            process.clone(),
        )));
        let end = EndSignals {
            exit: self.exit,
            stderr_done,
        };
        tasks.push(tokio::spawn(run_reader(stdout, mux.clone(), end)));
        Multiplexer {
            mux,
            tasks: Arc::new(Tasks::new(tasks, None)),
        }
    }
}

/// One multiplexed server process. Cheap to clone; channels keep the process tasks alive too.
#[derive(Clone)]
pub struct Multiplexer {
    mux: Arc<MuxShared>,
    tasks: Arc<Tasks>,
}

struct MuxShared {
    config: ConnectionConfig,
    /// Process-level state: stdin writer queue, stderr ring, death, frame counters. No request is ever
    /// sent on it (an unwrapped frame would end the server process).
    process: Arc<Shared>,
    handlers: Vec<(String, Arc<dyn ServerRequestHandler>)>,
    /// Open channels by id. A channel leaves when the server's close frame arrives or the process dies.
    channels: Mutex<HashMap<u64, Arc<Shared>>>,
    last_id: AtomicU64,
    unknown_channel_frames: AtomicU64,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Multiplexer {
    /// Opens the next channel and returns its connection. Nothing is sent: the server creates its side
    /// on the channel's first frame (the caller's `initialize`; every channel needs its own handshake,
    /// Vibe answers `not_initialized` otherwise).
    ///
    /// # Errors
    /// [`RpcError::ServerDead`] when the process is dead or closing, or (cause `ChannelClosed`) when the
    /// id space is exhausted ([`MAX_CHANNEL_ID`] channels in one process: not reachable in practice).
    pub fn open_channel(&self) -> Result<Connection, RpcError> {
        let mux = &self.mux;
        // Death is checked under the table lock: `fail_all` takes the same lock after the process is
        // marked dead, so a new channel is either failed there or refused here, never left alive.
        let mut table = locked(&mux.channels);
        if let Some(report) = mux.process.death() {
            return Err(RpcError::ServerDead(Box::new(report)));
        }
        if mux.process.is_closing() {
            return Err(RpcError::ServerDead(Box::new(dead_report(
                &mux.process,
                DeathCause::ClosedByClient,
            ))));
        }
        let id = mux.last_id.fetch_add(1, Ordering::Relaxed) + 1;
        if id > MAX_CHANNEL_ID {
            return Err(RpcError::ServerDead(Box::new(dead_report(
                &mux.process,
                DeathCause::ChannelClosed {
                    error: Some("channel ids exhausted in this process".to_owned()),
                },
            ))));
        }
        let (tx, rx) = mpsc::channel(mux.config.outbound_queue.max(1));
        let channel = Arc::new(Shared::new_channel(
            mux.config.clone(),
            tx,
            mux.process.stderr.clone(),
            id,
        ));
        for (method, handler) in &mux.handlers {
            channel.handlers.insert(method.clone(), handler.clone());
        }
        table.insert(id, channel.clone());
        drop(table);
        debug!(channel_id = id, "channel opened");
        let writer = tokio::spawn(run_channel_writer(
            id,
            rx,
            channel.clone(),
            mux.process.clone(),
        ));
        let on_drop = CloseOnDrop {
            id,
            channel: channel.clone(),
            process: mux.process.clone(),
        };
        let tasks = Tasks::new(vec![writer], Some(self.tasks.clone())).with_guard(on_drop);
        Ok(Connection::from_parts(channel, Arc::new(tasks)))
    }

    /// The process's death report, if it is dead.
    #[must_use]
    pub fn death(&self) -> Option<DeathReport> {
        self.mux.process.death()
    }

    /// A watch that becomes `Some(report)` when the process dies.
    #[must_use]
    pub fn death_watch(&self) -> watch::Receiver<Option<DeathReport>> {
        self.mux.process.death_watch()
    }

    /// Waits until the process is dead and returns the report.
    pub async fn closed(&self) -> DeathReport {
        let mut watch = self.mux.process.death_watch();
        loop {
            if let Some(report) = watch.borrow_and_update().clone() {
                return report;
            }
            if watch.changed().await.is_err() {
                return self.mux.process.mark_dead(DeathCause::ClosedByClient, None);
            }
        }
    }

    /// Closes the process's stdin: Vibe then closes every channel (each flushes its harness) and
    /// exits. New channels and requests are refused from now on.
    pub async fn close_input(&self) {
        self.mux.process.set_closing();
        let _ = self.mux.process.outbound.send(WriterCmd::Close).await;
    }

    /// The last `n` stderr lines of the process (oldest first).
    #[must_use]
    pub fn stderr_tail(&self, n: usize) -> Vec<String> {
        self.mux.process.stderr.tail(n)
    }

    /// Counter snapshot.
    #[must_use]
    pub fn stats(&self) -> MuxStats {
        let mux = &self.mux;
        let counters = &mux.process.counters;
        MuxStats {
            channels_opened: mux.last_id.load(Ordering::Relaxed),
            open_channels: locked(&mux.channels).len(),
            unknown_channel_frames: mux.unknown_channel_frames.load(Ordering::Relaxed),
            invalid_frames: counters.invalid_frames.load(Ordering::Relaxed),
            oversized_frames: counters.oversized_frames.load(Ordering::Relaxed),
        }
    }
}

/// A report for a refusal before anything was sent.
fn dead_report(process: &Shared, cause: DeathCause) -> DeathReport {
    DeathReport {
        cause,
        exit: None,
        stderr_tail: process.stderr.tail(process.config.death_stderr_lines),
    }
}

/// The outer frame around an encoded inner frame (`inner` ends with `\n`, from `encode_frame`).
fn wrap(channel_id: u64, inner: &[u8]) -> Vec<u8> {
    let body = inner.strip_suffix(b"\n").unwrap_or(inner);
    let head = format!("{{\"channelId\":\"{channel_id}\",\"message\":");
    let mut out = Vec::with_capacity(head.len() + body.len() + 2);
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(body);
    out.extend_from_slice(b"}\n");
    out
}

/// The frame that closes a channel.
fn close_frame(channel_id: u64) -> Vec<u8> {
    format!("{{\"channelId\":\"{channel_id}\",\"message\":null}}\n").into_bytes()
}

/// Closes the channel on the server when its last handle is dropped without `close_input`.
struct CloseOnDrop {
    id: u64,
    channel: Arc<Shared>,
    process: Arc<Shared>,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if self.channel.death().is_some() || self.channel.is_closing() {
            return;
        }
        self.channel.set_closing();
        // Non-blocking: a full process queue means the process is stuck, and its death closes all.
        if self
            .process
            .outbound
            .try_send(WriterCmd::Frame(close_frame(self.id)))
            .is_err()
        {
            debug!(
                channel_id = self.id,
                "could not queue the close of a dropped channel"
            );
        }
    }
}

/// Forwards one channel's frames to the process writer, wrapped. `Close` sends the close frame and
/// ends the task. A channel that dies locally on a wedge (an unanswered `callback/result`) is closed on
/// the server too, so its harness releases the session.
async fn run_channel_writer(
    id: u64,
    mut queue: mpsc::Receiver<WriterCmd>,
    channel: Arc<Shared>,
    process: Arc<Shared>,
) {
    let mut death = channel.death_watch();
    loop {
        tokio::select! {
            cmd = queue.recv() => {
                let bytes = match cmd {
                    Some(WriterCmd::Frame(bytes)) => wrap(id, &bytes),
                    Some(WriterCmd::Close) => {
                        let _ = process.outbound.send(WriterCmd::Frame(close_frame(id))).await;
                        return;
                    }
                    None => return,
                };
                if process.outbound.send(WriterCmd::Frame(bytes)).await.is_err() {
                    let cause = process
                        .death()
                        .map_or(DeathCause::WriteFailed("process writer gone".to_owned()), |r| r.cause);
                    channel.mark_dead(cause, None);
                    return;
                }
            }
            changed = death.changed() => {
                let wedged = matches!(
                    death.borrow().as_ref().map(|r| &r.cause),
                    Some(DeathCause::Wedged { .. })
                );
                if wedged {
                    channel.set_closing();
                    let _ = process.outbound.send(WriterCmd::Frame(close_frame(id))).await;
                }
                if changed.is_err() || death.borrow().is_some() {
                    return;
                }
            }
        }
    }
}

/// The process reader: one outer frame per line, routed to its channel.
async fn run_reader<R>(stdout: R, mux: Arc<MuxShared>, end: EndSignals)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::with_capacity(64 * 1024, stdout);
    // Handler tasks of every channel; aborted when the reader ends (process gone).
    let mut handlers = JoinSet::new();
    let limit = mux.config.max_frame_bytes.saturating_add(WRAPPER_BYTES);
    let cause = loop {
        while handlers.try_join_next().is_some() {}
        match read_bounded_line(&mut reader, limit).await {
            Ok(BoundedLine::Line(bytes)) => mux.route_line(&mut handlers, &bytes),
            Ok(BoundedLine::Overflow { discarded }) => {
                bump(&mux.process.counters.oversized_frames);
                warn!(discarded, limit, "discarded an oversized multiplexed frame");
                mux.gap_all(&GapReason::FrameTooLarge { discarded, limit });
            }
            Ok(BoundedLine::Eof) => break DeathCause::Eof,
            Err(error) => break DeathCause::ReadFailed(error.to_string()),
        }
    };
    let exit = reader::wait_for_end(end, mux.config.exit_grace).await;
    let cause = if cause == DeathCause::Eof && mux.process.is_closing() {
        DeathCause::ClosedByClient
    } else {
        cause
    };
    let report = mux.process.mark_dead(cause, exit);
    mux.fail_all(&report);
}

impl MuxShared {
    /// Parses one outer frame and hands its message to the channel (never awaits).
    fn route_line(&self, handlers: &mut JoinSet<()>, bytes: &[u8]) {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let value: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(error) => return self.invalid_outer(format!("not JSON: {error}")),
        };
        let Value::Object(mut frame) = value else {
            return self.invalid_outer("frame is not a JSON object".to_owned());
        };
        let id = match frame.remove("channelId") {
            Some(Value::String(text)) => parse_channel_id(&text),
            _ => None,
        };
        let Some(id) = id else {
            return self.invalid_outer("frame has no valid channelId".to_owned());
        };
        let channel = locked(&self.channels).get(&id).cloned();
        let Some(channel) = channel else {
            self.unknown_channel_frames.fetch_add(1, Ordering::Relaxed);
            warn!(
                channel_id = id,
                "frame for a channel that is not open; dropped"
            );
            return;
        };
        match frame.remove("message") {
            Some(message @ Value::Object(_)) => reader::dispatch_value(&channel, handlers, message),
            Some(Value::Null) | None => {
                let error = frame
                    .remove("error")
                    .and_then(|e| e.as_str().map(str::to_owned));
                self.server_closed(id, &channel, error);
            }
            Some(_) => reader::invalid(&channel, "channel message is not an object".to_owned()),
        }
    }

    /// The server closed a channel: our close acknowledged, or its own decision.
    fn server_closed(&self, id: u64, channel: &Shared, error: Option<String>) {
        locked(&self.channels).remove(&id);
        let ours = channel.is_closing() && error.is_none();
        let cause = if ours {
            DeathCause::ClosedByClient
        } else {
            DeathCause::ChannelClosed { error }
        };
        debug!(channel_id = id, cause = cause.kind(), "channel closed");
        channel.mark_dead(cause, None);
    }

    /// A line that is not a valid channel frame: any channel may have lost a message.
    fn invalid_outer(&self, reason: String) {
        bump(&self.process.counters.invalid_frames);
        // The line content is never logged: it may carry prompts or file contents.
        warn!(%reason, "discarded an invalid multiplexed frame");
        self.gap_all(&GapReason::InvalidFrame { reason });
    }

    fn gap_all(&self, reason: &GapReason) {
        let channels: Vec<Arc<Shared>> = locked(&self.channels).values().cloned().collect();
        let item = Inbound::Gap(reason.clone());
        for channel in channels {
            channel.fan_out(&item);
        }
    }

    /// The process died: every channel dies with the same cause and exit.
    fn fail_all(&self, report: &DeathReport) {
        let channels: Vec<Arc<Shared>> = locked(&self.channels).drain().map(|(_, c)| c).collect();
        for channel in channels {
            channel.mark_dead(report.cause.clone(), report.exit);
        }
    }
}

/// A channel id as the server writes it: decimal, no leading zero, 1..=16 digits.
fn parse_channel_id(text: &str) -> Option<u64> {
    let digits_ok = (1..=16).contains(&text.len())
        && text.bytes().all(|b| b.is_ascii_digit())
        && !text.starts_with('0');
    digits_ok.then(|| text.parse().ok()).flatten()
}
