//! Cleans up a failed manual compaction without closing a newer session channel.
//! Depends on: `channels`, `CoreError`, the gated request failure. Called by: `dispatch`.

use serde_json::Value;
use vw_gate::GateError;

use crate::channels::ChannelKey;
use crate::core::Shared;
use crate::error::CoreError;

pub(crate) fn channel(shared: &Shared, method: &str, params: &Value) -> Option<ChannelKey> {
    (method == "session/compact")
        .then(|| {
            params["sessionId"]
                .as_str()
                .and_then(|id| shared.channels.key_of(id))
        })
        .flatten()
}

pub(crate) async fn failed(
    shared: &Shared,
    method: &str,
    params: &Value,
    channel: Option<ChannelKey>,
    error: &GateError,
) -> CoreError {
    if method == "session/compact"
        && let (Some(id), Some(key)) = (params["sessionId"].as_str(), channel)
    {
        shared.channels.close_if(id, key).await;
    }
    let mut failure = CoreError::from_gate(method, error);
    if method == "session/compact" {
        failure.message = format!(
            "Context compaction failed: {}. Send another message to continue.",
            failure.message
        );
    }
    failure
}
