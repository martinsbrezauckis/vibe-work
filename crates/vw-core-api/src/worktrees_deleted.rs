//! Confirmed chat deletion through the gate and per-chat lock.
//! Depends on session deletion bookkeeping and turn dispatch; called by the dispatcher.

use serde_json::Value;
use vw_gate::{ConfirmationToken, GateError, Reply};

use crate::core::Shared;

/// Keep deletion bookkeeping inside the exact confirmed request.
pub(crate) async fn call(
    shared: &std::sync::Arc<Shared>,
    method: &str,
    params: &Value,
    token: Option<&ConfirmationToken>,
    stop: Option<u64>,
) -> Result<Reply, GateError> {
    if method == "session/delete" {
        shared.gate.check(method, params, token)?;
    }
    let id = params["sessionId"]
        .as_str()
        .filter(|_| method == "session/delete");
    let one = if let Some(id) = id {
        Some(shared.deletes.lock(id).await)
    } else {
        None
    };
    let fallback = if let Some(id) = id {
        crate::session_delete::prepare(shared, id).await
    } else {
        None
    };
    let result = crate::turn_start::call(shared, method, params, token, stop).await;
    let result = if let Some(id) = id {
        crate::session_delete::finish(shared, id, fallback, result, one).await
    } else {
        result
    };
    // Vibe's delete leaves the chat's row (title, first-message preview) in `unified/.session-index.json` until its
    // next session/list rebuilds that index: list once, so a deleted chat leaves no copy of its words behind.
    if id.is_some() && result.is_ok() {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            shared
                .gate
                .call("session/list", serde_json::json!({"limit": 1}), None),
        )
        .await;
    }
    crate::session_end::missing_reply(shared, method, params, result)
}
