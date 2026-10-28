//! Chat-owned schedules: clear Vibe's copied /new loops and refresh list rows after loop changes.
//! Uses only exact loop ids and gated Vibe calls; dispatch calls this before publishing its result.
use crate::{core::Shared, error::CoreError};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) async fn after(
    shared: &Arc<Shared>,
    method: &str,
    params: &Value,
    result: &mut Value,
) -> Result<(), CoreError> {
    if method == "session/history/clear" {
        let Some(new) = result["state"]["session"]["id"].as_str().map(str::to_owned) else {
            return Ok(());
        };
        if params["sessionId"] == new {
            return Ok(());
        }
        // A chat started by `/new` begins with no scheduled loops: each loop its starting state lists is removed
        // before the new chat is shown.
        let copied = result["state"]["session"]["loops"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for item in &copied {
            if let Some(id) = item["id"].as_str()
                && let Err(error) = delete(shared, &new, id).await
            {
                shared.channels.close(&new).await;
                return Err(error);
            }
        }
        result["state"]["session"]["loops"] = json!([]);
    }
    // Only a change re-reads the chat (Vibe returns loop changes without a session update). `loops/list` is a read:
    // the UI calls it on every chat open and turn end, and the row mark uses that answer, so re-reading the session
    // after it would be a `session/read` poll while chats merely run (sessions_e2e P7).
    if matches!(method, "loops/create" | "loops/delete" | "loops/clear") {
        let Some(session) = params["sessionId"].as_str() else {
            return Ok(());
        };
        refresh(shared, session).await;
    }
    Ok(())
}

async fn delete(shared: &Arc<Shared>, session: &str, id: &str) -> Result<(), CoreError> {
    if !shared.channels.has_channel(session) {
        let core = crate::Core {
            shared: shared.clone(),
        };
        Box::pin(core.attach_session(crate::Origin::Ui, session)).await?;
    }
    shared
        .gate
        .call(
            "loops/delete",
            json!({"sessionId":session,"loopId":id}),
            None,
        )
        .await
        .map_err(|e| CoreError::from_gate("loops/delete", &e))?;
    Ok(())
}

async fn refresh(shared: &Shared, session: &str) {
    if let Some(applied) = crate::pump::refresh(shared, session).await {
        let _ = shared.loads.send(applied).await;
    }
}
