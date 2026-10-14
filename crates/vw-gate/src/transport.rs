//! The gate's only way to reach Vibe: a one-method trait, implemented for the real connection and
//! for test fakes that record every frame.
//!
//! Depends on: vw-rpc (`Connection`, `RpcError`). Called by: gate.
//! Invariant: the gate never calls anything on the connection except `request`.

use std::future::Future;

use serde_json::Value;
use vw_rpc::{Connection, RpcError};

/// Sends one JSON-RPC request and returns its result.
pub trait Transport: Send + Sync {
    /// Sends `method` with `params` and waits for the answer.
    ///
    /// # Errors
    /// Whatever the underlying connection reports (remote error, timeout, dead server).
    fn request(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, RpcError>> + Send;
}

impl Transport for Connection {
    fn request(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, RpcError>> + Send {
        Connection::request(self, method, params)
    }
}
