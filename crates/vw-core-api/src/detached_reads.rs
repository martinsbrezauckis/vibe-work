//! Reads for a detached chat use stored state, plugin-free lobby config, or an empty runtime-only list.
//! Called under dispatch's chat lock after gate admission; never opens a channel or runs old work.
//! Depends on: the store, method gate and lobby transport.
//! Called by: command forwarding under the chat lock.
use serde_json::{Value, json};
use vw_gate::{ConfirmationToken, Reply};

use crate::{core::Shared, error::CoreError};

/// A detached runtime is absent, not a failed read. Attached chats retain Vibe's live responses.
pub(crate) async fn answer(
    shared: &Shared,
    method: &str,
    params: &Value,
    token: Option<&ConfirmationToken>,
) -> Result<Option<Reply>, CoreError> {
    let Some(id) = params["sessionId"].as_str() else {
        return Ok(None);
    };
    if shared.channels.has_channel(id) || !handled(method) {
        return Ok(None);
    }
    shared
        .gate
        .check(method, params, token)
        .map_err(|e| CoreError::from_gate(method, &e))?;
    // A raw read may precede ViewSession. Probe the store, never resume, and preserve genuine not-found errors.
    let state = if let Some(view) = shared.store.view(id) {
        view.state
    } else {
        shared
            .gate
            .call("session/read", json!({"sessionId":id}), None)
            .await
            .map_err(|e| CoreError::from_gate("session/read", &e))?
            .result["state"]
            .clone()
    };
    let session = &state["session"];
    let log = json!({"enabled":true,"sessionId":id,"persisted":true,"title":session["title"]});
    let result = match method {
        "runtime/read" => {
            json!({"runtime":crate::detached_runtime::read(shared, id, &state).await?,
            "sessionLog":log,"ready":false})
        }
        "config/read" => json!({"config":crate::detached_runtime::config(shared, &state).await?}),
        "loops/list" => json!({"loops":session.get("loops").cloned().unwrap_or(json!([]))}),
        "skills/list" | "skills/installed" => json!({"skills":[]}),
        "skills/catalog" => json!({"skills":[],"loaded":false}),
        "skills/updates" => json!({"updates":[]}),
        "skills/detail" => json!({"detail":null,"body":null}),
        "skills/versions" => json!({"versions":[]}),
        "session/turn/queue/read" => json!({"queue":state.get("turnQueue").cloned()
            .unwrap_or(json!({"items":[],"paused":true}))}),
        "config/fields/read" => json!({"fields":[],"targets":[]}),
        "config/proxy/read" => json!({"settings":{"values":{},"descriptions":{}}}),
        "account/read" => json!({"detached":true,"account":{"status":"unavailable"}}),
        "identity/read" => json!({"identity":null}),
        "session/log/read" => json!({"log":log}),
        "session/subagents/read" => json!({"subagents":[]}),
        "session/rewind/read" => json!({"hasFileChanges":false,"paths":[]}),
        "session/ready/read" => json!({"ready":false}),
        "session/context/read" => json!({"breakdown":{"contextWindow":0,"compactionThreshold":0,
            "usedTokens":0,"totalCharacters":0,"measured":false,"categories":[]}}),
        "plugin_catalog/read" => json!({"plugins":{"plugins":[],"dropped":[]}}),
        "mcp_catalog/read" => json!({"mcp":{"sources":[]}}),
        "connector_catalog/read" => json!({"catalog":{"disposition":"not_loaded","connectors":[]},
            "selections":[]}),
        _ => unreachable!("handled read"),
    };
    Ok(Some(Reply {
        result,
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
    }))
}

fn handled(method: &str) -> bool {
    matches!(
        method,
        "runtime/read"
            | "config/read"
            | "loops/list"
            | "skills/list"
            | "skills/installed"
            | "skills/catalog"
            | "skills/updates"
            | "skills/detail"
            | "skills/versions"
            | "session/turn/queue/read"
            | "config/fields/read"
            | "config/proxy/read"
            | "account/read"
            | "identity/read"
            | "session/log/read"
            | "session/subagents/read"
            | "session/rewind/read"
            | "session/ready/read"
            | "session/context/read"
            | "plugin_catalog/read"
            | "mcp_catalog/read"
            | "connector_catalog/read"
    )
}
