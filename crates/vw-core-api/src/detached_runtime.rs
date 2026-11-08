//! Saved composer selections over the lobby's plugin-free configuration for detached chat reads.
//! Called by `detached_reads`; a display snapshot is never published as an attached runtime.
use crate::{core::Shared, error::CoreError};
use serde_json::{Value, json};

pub(crate) async fn config(shared: &Shared, state: &Value) -> Result<Value, CoreError> {
    // A lobby read for the saved folder, so no channel opens for the stored chat. It still loads the plugins
    // root, and it answers the chat's own config/read (which `loads_plugins` sweeps for): sweep first, so the
    // server never sees an unapproved plugin folder (vw_plugin_boundaries).
    shared.channels.sweep_plugins().await?;
    let params = state["session"]["cwd"]
        .as_str()
        .map_or_else(|| json!({}), |cwd| json!({"cwd":cwd}));
    let mut config = shared
        .gate
        .call("config/read", params, None)
        .await
        .map_err(|e| CoreError::from_gate("config/read", &e))?
        .result["config"]
        .clone();
    let session = &state["session"];
    if let Some(alias) = session["model"].as_str() {
        let model = config["models"].as_array().and_then(|models| models.iter()
            .find(|model| model["alias"] == alias)).cloned().unwrap_or_else(|| json!({
                "alias":alias,"name":alias,"displayName":alias,
                "thinking":session["reasoningEffort"].as_str().unwrap_or_default(),"supportsImages":false
            }));
        config["activeModel"] = model;
        config["activeModelPinned"] = json!(true);
        if let Some(effort) = session["reasoningEffort"].as_str() {
            config["activeModel"]["thinking"] = json!(effort);
        }
    }
    Ok(config)
}

pub(crate) async fn read(shared: &Shared, id: &str, state: &Value) -> Result<Value, CoreError> {
    if let Some(runtime) = shared.store.runtime(id) {
        return Ok(runtime);
    }
    let config = config(shared, state).await?;
    let agent = state["session"].get("agent").filter(|agent| agent.is_object()).cloned()
        .unwrap_or_else(|| {
            let name = crate::restore::display_mode(shared, id, &state["session"])
                .unwrap_or_else(|| crate::access::FALLBACK_PROFILE.into());
            json!({"name":name,"displayName":name,"description":"","safety":"neutral","agentType":"agent"})
        });
    let context_window = config["activeModel"]["maxContextLength"]
        .as_i64()
        .unwrap_or(0);
    Ok(
        json!({"config":config,"activeAgent":agent,"agents":[agent],"skills":[],"tools":[],
        "stats":{},"contextWindow":context_window,"issues":[],"hooksCount":0,"connectors":{},"mcp":{}}),
    )
}
