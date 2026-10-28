//! Cleans up a failed manual compaction without closing a newer session channel, and lets the chat's Stop reach a
//! running compaction.
//! Depends on: `channels`, `stops` (the in-flight count), `plugin_refresh` (the chat's send lock), `CoreError`, the
//! gated request failure. Called by: `dispatch` (the forward and `stopSession`).
//! Invariant: a `session/compact` forward holds the chat's send lock until Vibe answers; while one is in flight no
//! other send can be mid-way (it would need that lock), so the Stop closes the chat without it and never waits for
//! the whole compaction. Otherwise the Stop takes the lock as before (`stops.rs` ordering). A send after the
//! Stop waits until a compaction in flight has finished, so no message is lost.

use serde_json::Value;
use vw_gate::GateError;

use crate::channels::ChannelKey;
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// The channel and Stop generation that own a pending compaction.
pub(crate) struct Compaction {
    key: ChannelKey,
    generation: u64,
}

/// The channel a `session/compact` is sent on, and its in-flight mark (held until the forward returns).
pub(crate) fn channel<'a>(
    shared: &'a Shared,
    method: &str,
    params: &Value,
) -> (Option<Compaction>, Option<InFlight<'a>>) {
    let key = (method == "session/compact")
        .then(|| {
            params["sessionId"].as_str().and_then(|id| {
                shared.channels.key_of(id).map(|key| Compaction {
                    key,
                    generation: crate::stops::generation(shared, id),
                })
            })
        })
        .flatten();
    (key, in_flight(shared, method, params))
}

pub(crate) async fn failed(
    shared: &Shared,
    method: &str,
    params: &Value,
    channel: Option<Compaction>,
    error: &GateError,
) -> CoreError {
    if method == "session/compact"
        && let (Some(id), Some(key)) = (params["sessionId"].as_str(), channel)
    {
        // A client-close error is expected only when this request was actually overtaken by Stop.
        // Inspect before cleanup: closing a failed request ourselves does not make it a person's Stop.
        let stopped = crate::stops::generation(shared, id) != key.generation
            && matches!(error,
            GateError::Transport(vw_rpc::RpcError::ServerDead(report))
            if report.cause == vw_rpc::DeathCause::ClosedByClient);
        shared.channels.close_if(id, key.key).await;
        if stopped {
            let mut cancelled = CoreError::new(
                ErrorCode::Cancelled,
                "Still compacting. Your next message is sent after that.",
            );
            cancelled.method = Some(method.into());
            cancelled.server_code = Some("compactionStopped".into());
            return cancelled;
        }
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

/// Marks a `session/compact` forward as in flight until dropped. Taken while the forward holds the send lock.
pub(crate) struct InFlight<'a>(&'a Shared, String);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        crate::stops::note_compaction(self.0, &self.1, false);
    }
}

fn in_flight<'a>(shared: &'a Shared, method: &str, params: &Value) -> Option<InFlight<'a>> {
    let id = params["sessionId"]
        .as_str()
        .filter(|_| method == "session/compact")?;
    crate::stops::note_compaction(shared, id, true);
    Some(InFlight(shared, id.to_owned()))
}

/// The chat's send lock for its Stop, or `None` once a compaction holds it (checked again every 50 ms while the
/// Stop waits, so a compaction that took the lock first is not waited for either).
pub(crate) async fn stop_lock(
    shared: &Shared,
    canonical: &str,
) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    loop {
        if crate::stops::compacting(shared, canonical) {
            return None;
        }
        tokio::select! {
            held = shared.plugin_refresh.lock(canonical) => return Some(held),
            () = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }
    }
}
