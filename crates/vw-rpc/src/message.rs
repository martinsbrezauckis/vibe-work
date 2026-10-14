//! JSON-RPC 2.0 message shapes on the wire: request ids, inbound classification, outbound frames.
//!
//! Depends on: `serde_json`, vw-protocol (`NotificationMethod`, `ProtocolError`). Called by: the
//! connection reader and writer.
//! Invariants: request ids come from one process-wide counter and are never reused in this
//! process, across connections and restarts (a late answer from a dead server can never match a
//! new request); every frame we send carries `"jsonrpc": "2.0"`.

use std::sync::atomic::{AtomicI64, Ordering};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use vw_protocol::{NotificationMethod, ProtocolError, ProtocolErrorCode};

static NEXT_REQUEST_ID: AtomicI64 = AtomicI64::new(1);

/// Allocates the next request id (process-wide, strictly increasing, starts at 1).
pub(crate) fn next_request_id() -> i64 {
    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

/// A server notification, delivered to subscribers.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// Method name, e.g. `turn/completed`.
    pub method: String,
    /// The `params` value (`{}` when the server sent none).
    pub params: Value,
}

impl Notification {
    /// Decodes the params as the typed params of notification `N`.
    ///
    /// # Errors
    /// Returns the decode error when the params do not match `N::Params`. A method-name mismatch
    /// is not checked here; match on [`Notification::method`] first.
    pub fn decode<N>(&self) -> Result<N::Params, serde_json::Error>
    where
        N: NotificationMethod,
        N::Params: DeserializeOwned,
    {
        serde_json::from_value(self.params.clone())
    }
}

/// Why subscribers may have missed protocol data. A subscriber that receives a gap must assume
/// its view of server state is stale and re-read it (e.g. `session/read`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapReason {
    /// A frame longer than the configured limit was discarded.
    FrameTooLarge {
        /// Bytes discarded.
        discarded: u64,
        /// The frame limit in force.
        limit: usize,
    },
    /// A line that was not a JSON object was discarded (it may have been a notification).
    InvalidFrame {
        /// Short reason (parse error or shape problem), never the line content.
        reason: String,
    },
}

/// One item on a subscription.
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    /// A notification from the server.
    Notification(Notification),
    /// Protocol data may have been lost; resync.
    Gap(GapReason),
}

/// A request the server sent to us (`callback/call`, `clientTool/*`); it must be answered.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerRequest {
    /// The server's id, echoed back verbatim (Vibe may use numbers or strings,
    /// `protocol.py:2346 JsonRpcServerRequest`).
    pub id: Value,
    /// Method name.
    pub method: String,
    /// The `params` value (`{}` when absent).
    pub params: Value,
    /// The channel it arrived on, for a channel of a multiplexed process (`None` on a plain
    /// connection). Lets a shared handler check that a request belongs to its channel's session.
    pub channel_id: Option<u64>,
}

/// Classified inbound line.
#[derive(Debug)]
pub(crate) enum Classified {
    Response {
        id: Value,
        outcome: Result<Value, Value>,
    },
    Request(ServerRequest),
    Notification(Notification),
    Invalid(String),
}

/// Classifies one parsed line. Order follows JSON-RPC: `id` + `method` is a server request, `id`
/// alone a response, `method` alone a notification (same order as ShellX
/// app-server provider @ e38c2b5c).
pub(crate) fn classify(value: Value) -> Classified {
    let Value::Object(mut object) = value else {
        return Classified::Invalid("frame is not a JSON object".to_owned());
    };
    let method = match object.remove("method") {
        None => None,
        Some(Value::String(method)) => Some(method),
        Some(_) => return Classified::Invalid("method is not a string".to_owned()),
    };
    let id = object.remove("id");
    let params = object
        .remove("params")
        .unwrap_or_else(|| Value::Object(Map::new()));
    match (id, method) {
        (Some(id), Some(method)) => Classified::Request(ServerRequest {
            id,
            method,
            params,
            channel_id: None,
        }),
        (Some(id), None) => {
            if let Some(error) = object.remove("error") {
                Classified::Response {
                    id,
                    outcome: Err(error),
                }
            } else if let Some(result) = object.remove("result") {
                Classified::Response {
                    id,
                    outcome: Ok(result),
                }
            } else {
                Classified::Invalid("response has neither result nor error".to_owned())
            }
        }
        (None, Some(method)) => Classified::Notification(Notification { method, params }),
        (None, None) => Classified::Invalid("frame has neither id nor method".to_owned()),
    }
}

/// Builds a request frame.
pub(crate) fn request_frame(id: i64, method: &str, params: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// Builds a notification frame.
pub(crate) fn notification_frame(method: &str, params: &Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

/// Builds the answer to a server request.
pub(crate) fn reply_frame(id: &Value, outcome: &Result<Value, ProtocolError>) -> Value {
    match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    }
}

/// Converts a raw error object into a typed [`ProtocolError`]. Vibe uses string codes
/// (`protocol.py:2353`); anything else (e.g. a numeric code from a non-Vibe peer) is kept as an
/// unrecognized code with the raw JSON as data, never dropped.
pub(crate) fn protocol_error(raw: Value) -> ProtocolError {
    if let Ok(error) = serde_json::from_value::<ProtocolError>(raw.clone()) {
        return error;
    }
    let code = raw.get("code").map_or_else(
        || "unknown".to_owned(),
        |code| match code {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        },
    );
    let message = raw
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("error without message")
        .to_owned();
    ProtocolError {
        code: ProtocolErrorCode::Unrecognized(code),
        message,
        data: raw.into(),
    }
}
