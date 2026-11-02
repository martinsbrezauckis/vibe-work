//! Static Chrome MCP definitions, advertised only when the core enables the catalog at session launch.
//! Depends on `serde_json`; called by `mcp`. The real-helper drift test pins names, descriptions and schemas.

use serde_json::{Value, json};

pub(crate) fn tool_list() -> Vec<Value> {
    // Native discovery cannot render a root oneOf; the gateway validates action/args versus steps.
    vec![
        json!({
            "name": "chrome_status",
            "description": "Connected Chrome profiles, chat sessions and action IDs. An action argument returns its input schema.",
            "inputSchema": schema(&json!({"action":{"type":"string"}}), &[]),
        }),
        json!({
            "name": "chrome_session",
            "description": "Open or inspect this chat’s Chrome window, cancel its current work or stop its session. Stop leaves the window open.",
            "inputSchema": schema(&json!({
                "action": {"enum": ["open", "get", "cancel", "stop"]},
                "sessionId": {"type": "string"},
                "profileId": {"type": "string"},
                "purpose": {"type": "string"},
                "tabId": {"type": "integer"},
                "route": {"enum": ["extension"]},
            }), &["action"]),
        }),
        json!({
            "name": "chrome_execute",
            "description": "Run an action or up to 16 sequential steps in this chat’s Chrome session. Each step uses the current chat mode. A batch stops at the first refusal, failure, unknown outcome or 25-second deadline. Files land in the chat folder.",
            "inputSchema": schema(&json!({
                "sessionId": {"type": "string"},
                "action": {"enum": [
                    "page.read",
                    "page.click",
                    "page.type",
                    "page.select",
                    "page.key",
                    "page.scroll",
                    "page.wait",
                    "page.navigate",
                    "page.commit",
                    "tabs.inspect_other",
                    "tabs.create",
                    "tabs.close_owned",
                    "tabs.close_user",
                    "capture.screenshot",
                    "bookmarks.read",
                    "bookmarks.edit",
                    "bookmarks.remove",
                    "history.read",
                    "history.delete",
                    "files.download",
                    "files.list_downloads",
                    "files.upload",
                    "cookies.metadata",
                    "cookies.change",
                    "storage.metadata",
                    "storage.read",
                    "storage.change",
                    "raw.execute",
                    "page.hover",
                    "page.double_click",
                    "page.right_click",
                    "page.drag",
                    "page.dialogs",
                    "page.dialog.dismiss",
                    "page.dialog.accept",
                    "page.dialog.prompt",
                    "page.back",
                    "page.forward",
                    "page.reload",
                    "page.errors",
                    "files.save_pdf",
                    "cookies.read",
                    "tabs.list",
                    "auth.fill",
                    "console.read",
                    "network.list",
                    "network.headers",
                    "network.body",
                    "page.triple_click",
                    "page.middle_click",
                    "page.check",
                    "page.uncheck",
                    "capture.zoom",
                    "recording.start",
                    "recording.stop",
                    "window.resize",
                    "webmcp.list",
                    "webmcp.call",
                    "files.drop",
                    "auth.basic",
                ]},
                "args": {"type": "object"},
                "steps": {"type":"array","minItems":1,"maxItems":16,"items":schema(&json!({"action":{"type":"string"},"args":{"type":"object"}}), &["action","args"])},
            }), &["sessionId"]),
        }),
    ]
}

fn schema(properties: &Value, required: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false, "properties":properties, "required":required})
}
