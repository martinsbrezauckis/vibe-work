//! The gate's transport in the running app: each request goes to the channel of the session it
//! names, or to the lobby channel of the supervisor's current server (`channels.rs`), so one gate
//! (with its tokens and open callbacks) serves every session and survives server restarts.
//!
//! Depends on: vw-gate (`Transport`), `channels`, vw-rpc. Called by: the gate (every forwarded
//! request).
//! Invariant: a request is written only to a channel that finished its `initialize` handshake;
//! when no server is ready within the wait, nothing is written and the error is
//! `RpcError::Timeout { id: 0, .. }` (id 0 is never a real request id; `CoreError::from_rpc` maps
//! it to `serverUnavailable`).

use std::future::Future;
use std::sync::Arc;

use serde_json::Value;
use vw_gate::Transport;
use vw_rpc::RpcError;

use crate::channels::Channels;

/// Sends gate requests on the right channel.
pub(crate) struct SupervisedTransport {
    pub(crate) channels: Arc<Channels>,
}

impl Transport for SupervisedTransport {
    fn request(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, RpcError>> + Send {
        let channels = self.channels.clone();
        let method = method.to_owned();
        async move { channels.request(&method, params).await }
    }
}
