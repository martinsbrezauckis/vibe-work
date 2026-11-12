//! Refresh chat list rows after Vibe changes their schedules.
//! Uses the session pump; dispatch calls this before publishing a loop mutation result.
use crate::core::Shared;
use serde_json::Value;
use std::sync::Arc;

pub(crate) async fn after(shared: &Arc<Shared>, method: &str, params: &Value) {
    // Only a change re-reads the chat (Vibe returns loop changes without a session update). `loops/list` is a read:
    // the UI calls it on every chat open and turn end, and the row mark uses that answer, so re-reading the session
    // after it would be a `session/read` poll while chats merely run (sessions_e2e P7).
    if matches!(method, "loops/create" | "loops/delete" | "loops/clear") {
        let Some(session) = params["sessionId"].as_str() else {
            return;
        };
        refresh(shared, session).await;
    }
}

async fn refresh(shared: &Shared, session: &str) {
    if let Some(applied) = crate::pump::refresh(shared, session).await {
        let _ = shared.loads.send(applied).await;
    }
}
