//! Static Chrome MCP definitions, advertised only when the core enables the catalog at session launch.
//! Depends on `serde_json`; called by `mcp`. The real-helper drift test pins names, descriptions and schemas.

use serde_json::{Value, json};

pub(crate) fn tool_list() -> Vec<Value> {
    vec![
        json!({
            "name": "chrome_status",
            "description": "Connected Chrome profiles, current tasks, permissions and supported action schemas. Call first; never guess profile IDs.",
            "inputSchema": schema(&json!({}), &[]),
        }),
        json!({
            "name": "chrome_task",
            "description": "Request or inspect an exact browser task; inspect policy; pause, cancel or stop. Pause/cancel/stop stay pending until acknowledged. Only the person can approve or resume.",
            "inputSchema": schema(&json!({
                "action": {"enum": ["request", "get", "policy", "pause", "cancel", "stop"]},
                "taskId": {"type": "string"},
                "profileId": {"type": "string"},
                "purpose": {"type": "string"},
                "initialTabId": {"type": "integer"},
                "route": {"enum": ["extension"]},
            }), &["action"]),
        }),
        json!({
            "name": "chrome_execute",
            "description": "Run one typed action in an approved task and receive its bounded, final browser result. Get action argument schemas from chrome_status. Never repeat an unknown outcome; operation IDs are generated here.",
            "inputSchema": schema(&json!({
                "taskId": {"type": "string"},
                "action": {"enum": [
                    "page.read", "page.click", "page.type", "page.select", "page.key", "page.scroll",
                    "page.wait", "page.navigate", "scope.add_origin", "page.commit", "tabs.inspect_other",
                    "tabs.create", "tabs.close_owned", "tabs.close_user", "capture.screenshot",
                    "bookmarks.read", "bookmarks.edit", "history.read", "history.delete", "files.download",
                    "files.list_downloads", "files.upload", "cookies.metadata", "cookies.change",
                    "storage.metadata", "storage.read", "storage.change", "raw.execute",
                ]},
                "args": {"type": "object"},
            }), &["taskId", "action", "args"]),
        }),
    ]
}

fn schema(properties: &Value, required: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false, "properties":properties, "required":required})
}
