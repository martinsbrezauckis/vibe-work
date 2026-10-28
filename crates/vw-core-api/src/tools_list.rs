//! `tools/list`, answered by Vibe Work: Vibe's unified harness has no such method (it answers `method_not_found`),
//! so the gate denies it. The answer comes from the session's `runtime/read`, in the old method's shape
//! (`{"tools": [{"name", "isCustom"}]}`): Vibe's own tools as Vibe names them, then the tools of each MCP server in
//! the same reply (plugin servers included) as `<server>.<tool>` with `isCustom: true`; tools Vibe reports as
//! turned off are left out. The model reaches MCP tools through Vibe's tool search, not under these names.
//!
//! Depends on: `dispatch` (`Core::forward`, so `runtime/read` passes the gate as any read). Called by: `dispatch`
//! (a `Call` of `tools/list`, from the window or the Debug API).

use serde_json::{Value, json};

use crate::command::{CommandResult, Origin};
use crate::core::Core;
use crate::error::CoreError;

/// The method answered here.
pub(crate) const METHOD: &str = "tools/list";

/// Reads `runtime/read` with the caller's params (`sessionId`) and answers the tool list.
pub(crate) async fn answer(
    core: &Core,
    origin: Origin,
    params: Value,
) -> Result<CommandResult, CoreError> {
    let reply = core.forward(origin, "runtime/read", params, None).await?;
    Ok(CommandResult::Reply {
        result: from_runtime(&reply.result["runtime"]),
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
    })
}

/// The tool list in a `runtime/read` snapshot.
pub(crate) fn from_runtime(runtime: &Value) -> Value {
    let mut tools: Vec<Value> = runtime["tools"].as_array().cloned().unwrap_or_default();
    for source in runtime
        .pointer("/mcp/sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(server) = source["name"].as_str() else {
            continue;
        };
        for tool in source["tools"].as_array().into_iter().flatten() {
            if let Some(name) = tool["name"].as_str()
                && tool["enabled"] != false
            {
                tools.push(json!({"name": format!("{server}.{name}"), "isCustom": true}));
            }
        }
    }
    json!({ "tools": tools })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_tools_then_each_servers_enabled_tools() {
        let runtime = json!({
            "tools": [{"name": "bash", "isCustom": false}],
            "mcp": {"sources": [
                {"name": "chrome_control", "status": "enabled", "tools": [
                    {"name": "chrome_status", "enabled": true},
                    {"name": "chrome_off", "enabled": false},
                    {"name": "chrome_open"}
                ]},
                {"name": "broken", "status": "failed", "tools": []}
            ]}
        });
        assert_eq!(
            from_runtime(&runtime),
            json!({"tools": [
                {"name": "bash", "isCustom": false},
                {"name": "chrome_control.chrome_status", "isCustom": true},
                {"name": "chrome_control.chrome_open", "isCustom": true}
            ]})
        );
    }

    #[test]
    fn a_snapshot_without_tools_gives_an_empty_list() {
        assert_eq!(from_runtime(&json!({})), json!({"tools": []}));
    }
}
