//! Handlers for server-to-client requests (`callback/call`, `clientTool/*`).
//!
//! Depends on: vw-protocol (`ProtocolError`, `ProtocolErrorCode`). Called by: the connection
//! reader, which runs every handler in its own task so a slow handler never blocks reading.
//! Invariants: every server request gets exactly one answer: the handler's result, its error, an
//! `internal_error` when the handler panics, or `method_not_found` when no handler is registered.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use serde_json::Value;
use vw_protocol::{Maybe, ProtocolError, ProtocolErrorCode};

use crate::message::ServerRequest;

/// Boxed future returned by a handler.
pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<Value, HandlerError>> + Send>>;

/// Answers one kind of server request.
///
/// Implemented for any `Fn(ServerRequest) -> impl Future<Output = Result<Value, HandlerError>>`.
/// A handler must not keep a strong [`crate::Connection`] (that would keep the connection's tasks
/// alive forever); capture a [`crate::WeakConnection`] instead.
pub trait ServerRequestHandler: Send + Sync + 'static {
    /// Produces the `result` (or error) for one request.
    fn handle(&self, request: ServerRequest) -> HandlerFuture;
}

impl<F, Fut> ServerRequestHandler for F
where
    F: Fn(ServerRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, HandlerError>> + Send + 'static,
{
    fn handle(&self, request: ServerRequest) -> HandlerFuture {
        Box::pin(self(request))
    }
}

/// An error answer to a server request, sent as the JSON-RPC `error` object.
#[derive(Debug, Clone, PartialEq)]
pub struct HandlerError {
    /// Error code (Vibe's string codes, `protocol.py:2353`).
    pub code: ProtocolErrorCode,
    /// Human-readable message (never secrets or file contents).
    pub message: String,
    /// Optional structured data.
    pub data: Option<Value>,
}

impl HandlerError {
    /// A new error with the given code and message.
    #[must_use]
    pub fn new(code: ProtocolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Converts into the wire error object.
    #[must_use]
    pub fn into_protocol(self) -> ProtocolError {
        ProtocolError {
            code: self.code,
            message: self.message,
            data: self.data.map_or(Maybe::Absent, Maybe::Value),
        }
    }
}

/// Handler registry, keyed by method name.
#[derive(Default)]
pub(crate) struct Handlers {
    map: RwLock<HashMap<String, Arc<dyn ServerRequestHandler>>>,
}

impl Handlers {
    pub(crate) fn insert(&self, method: String, handler: Arc<dyn ServerRequestHandler>) {
        self.map
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(method, handler);
    }

    pub(crate) fn get(&self, method: &str) -> Option<Arc<dyn ServerRequestHandler>> {
        self.map
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(method)
            .cloned()
    }
}

/// Runs the handler (or the no-handler answer) for one request and returns the outcome to send.
///
/// The handler runs in its own task so a panic is caught as a `JoinError` and answered with
/// `internal_error`; the task is aborted if this future is dropped (connection shutdown).
pub(crate) async fn answer(
    handler: Option<Arc<dyn ServerRequestHandler>>,
    request: ServerRequest,
) -> Result<Value, ProtocolError> {
    let Some(handler) = handler else {
        return Err(HandlerError::new(
            ProtocolErrorCode::MethodNotFound,
            format!("no handler for {} in this client", request.method),
        )
        .into_protocol());
    };
    let task = AbortOnDrop(tokio::spawn(handler.handle(request)));
    match task.join().await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => Err(error.into_protocol()),
        Err(join) => Err(HandlerError::new(
            ProtocolErrorCode::InternalError,
            if join.is_panic() {
                "client handler panicked"
            } else {
                "client handler was cancelled"
            },
        )
        .into_protocol()),
    }
}

/// Owns a spawned task and aborts it when dropped, so handler tasks never outlive the connection.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> AbortOnDrop<T> {
    async fn join(mut self) -> Result<T, tokio::task::JoinError> {
        (&mut self.0).await
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
