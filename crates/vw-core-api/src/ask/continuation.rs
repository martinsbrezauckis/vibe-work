//! Command completion metadata can wake an active agent wait; it never starts a turn.
use crate::{Core, core::Shared};
use serde_json::json;
use std::sync::Arc;

/// Completion metadata wakes an active model wait without starting an unsolicited idle turn. Returns what was sent
/// (`None`: nothing to wake).
pub(crate) async fn secret_finished(
    shared: Arc<Shared>,
    session: String,
    id: String,
) -> Option<Result<vw_gate::Reply, crate::CoreError>> {
    let text = format!(
        "Vibe Work's secret_run request {id} has finished. Read secret_run_status with that requestId in a NEW run_typescript call."
    );
    let view = shared.store.view(&session)?;
    let turn = view
        .state
        .pointer("/session/status/activeTurnId")?
        .as_str()?;
    let generation = crate::stops::turn_generation(&shared, &session, Some(turn));
    let params =
        json!({"sessionId":session,"expectedTurnId":turn,"message":[{"type":"text","text":text}]});
    Some(
        Core { shared }
            .forward_unless_stopped("turn/steer", params, generation)
            .await,
    )
}
