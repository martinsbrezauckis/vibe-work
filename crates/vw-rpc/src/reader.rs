//! The reader task: reads stdout frames and dispatches responses, server requests and
//! notifications.
//!
//! Depends on: `framing`, `message`, `handler`, `shared`. Called by: `connection` (spawned once
//! per connection).
//! Invariants:
//! - The reader never awaits a handler or a subscriber: handlers run in tasks owned by the
//!   reader's `JoinSet` (aborted when the reader ends), fan-out is non-blocking.
//! - A bad line (too long, not JSON, wrong shape) is counted, reported to subscribers as a gap
//!   and skipped; it never ends the connection.
//! - On stdout EOF the reader waits up to `exit_grace` for the exit status and the last stderr
//!   lines, then declares the server dead with all three.

use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncRead, BufReader};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout_at};
use tracing::{debug, warn};

use crate::error::{DeathCause, ExitInfo};
use crate::framing::{BoundedLine, encode_frame, read_bounded_line};
use crate::handler::answer;
use crate::message::{Classified, GapReason, Inbound, classify, reply_frame};
use crate::shared::{Shared, WriterCmd, bump};

/// Signals the reader can wait on after EOF.
pub(crate) struct EndSignals {
    /// Exit status of the server process, if the caller owns the process.
    pub(crate) exit: Option<watch::Receiver<Option<ExitInfo>>>,
    /// Becomes `true` when the stderr pump has read to end of stream.
    pub(crate) stderr_done: Option<watch::Receiver<bool>>,
}

pub(crate) async fn run<R>(stdout: R, shared: Arc<Shared>, end: EndSignals)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::with_capacity(64 * 1024, stdout);
    let mut handlers = JoinSet::new();
    let limit = shared.config.max_frame_bytes;
    let cause = loop {
        // Reap finished handler tasks. Never `select!` on the read: `read_until` is not
        // cancel-safe and a partially read frame would be lost.
        while handlers.try_join_next().is_some() {}
        match read_bounded_line(&mut reader, limit).await {
            Ok(BoundedLine::Line(bytes)) => handle_line(&shared, &mut handlers, &bytes),
            Ok(BoundedLine::Overflow { discarded }) => {
                bump(&shared.counters.oversized_frames);
                warn!(discarded, limit, "discarded an oversized frame");
                shared.fan_out(&Inbound::Gap(GapReason::FrameTooLarge { discarded, limit }));
            }
            Ok(BoundedLine::Eof) => break DeathCause::Eof,
            Err(error) => break DeathCause::ReadFailed(error.to_string()),
        }
    };
    let exit = wait_for_end(end, shared.config.exit_grace).await;
    let cause = if cause == DeathCause::Eof && shared.is_closing() {
        DeathCause::ClosedByClient
    } else {
        cause
    };
    shared.mark_dead(cause, exit);
}

fn handle_line(shared: &Arc<Shared>, handlers: &mut JoinSet<()>, bytes: &[u8]) {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return;
    }
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(error) => return invalid(shared, format!("not JSON: {error}")),
    };
    dispatch_value(shared, handlers, value);
}

/// Dispatches one parsed JSON-RPC message: a response to its waiter, a server request to its handler
/// (in a task of `handlers`), a notification to the subscribers. Never awaits. Used for plain
/// connections (after `handle_line` parsed the line) and for channels of a multiplexed process (the
/// process reader parsed the outer frame and hands over the inner message, `mux.rs`).
pub(crate) fn dispatch_value(shared: &Arc<Shared>, handlers: &mut JoinSet<()>, value: Value) {
    match classify(value) {
        Classified::Response { id, outcome } => shared.complete(&id, outcome),
        Classified::Notification(notification) => {
            bump(&shared.counters.notifications);
            shared.fan_out(&Inbound::Notification(notification));
        }
        Classified::Request(mut request) => {
            request.channel_id = shared.channel_id;
            bump(&shared.counters.server_requests);
            let handler = shared.handlers.get(&request.method);
            if handler.is_none() {
                bump(&shared.counters.unhandled_server_requests);
                warn!(
                    method = method_for_log(&request.method),
                    "server request without a handler"
                );
            }
            debug!(method = method_for_log(&request.method), "server request");
            let shared = shared.clone();
            handlers.spawn(async move {
                let id = request.id.clone();
                let outcome = answer(handler, request).await;
                send_reply(&shared, id, outcome).await;
            });
        }
        Classified::Invalid(reason) => invalid(shared, reason),
    }
}

/// The method name as it may appear in a log line: a method this protocol version declares
/// (`vw_protocol::SERVER_REQUEST_METHODS`) is logged as is; anything else is server-chosen text
/// and is replaced, so a broken or hostile server cannot write into our logs.
fn method_for_log(method: &str) -> &str {
    if vw_protocol::SERVER_REQUEST_METHODS.contains(&method) {
        method
    } else {
        "<undeclared method>"
    }
}

/// Counts a discarded frame and reports it to the subscribers as a gap.
pub(crate) fn invalid(shared: &Shared, reason: String) {
    bump(&shared.counters.invalid_frames);
    // The line content is never logged: it may carry prompts or file contents.
    warn!(%reason, "discarded an invalid frame");
    shared.fan_out(&Inbound::Gap(GapReason::InvalidFrame { reason }));
}

async fn send_reply(
    shared: &Shared,
    id: Value,
    outcome: Result<Value, vw_protocol::ProtocolError>,
) {
    match encode_frame(&reply_frame(&id, &outcome)) {
        // A send error means the writer is gone (server dead): nobody to answer.
        Ok(bytes) => {
            let _ = shared.outbound.send(WriterCmd::Frame(bytes)).await;
        }
        Err(error) => warn!(%error, "could not encode a reply to a server request"),
    }
}

pub(crate) async fn wait_for_end(end: EndSignals, grace: std::time::Duration) -> Option<ExitInfo> {
    let deadline = Instant::now() + grace;
    if let Some(mut done) = end.stderr_done {
        let _ = timeout_at(deadline, done.wait_for(|done| *done)).await;
    }
    let mut exit = end.exit?;
    match timeout_at(deadline, exit.wait_for(Option::is_some)).await {
        Ok(Ok(info)) => *info,
        _ => None,
    }
}
