//! Named extension diagnostics and task notifications through the frozen scoped API transport.
//! Missing grants stay explicit; a notification receipt names the notification and confirms its live ID.
use super::{owner, page_plan, read_consent::Consent};
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) const NAMES: &[&str] = &[
    "extensions.list",
    "extensions.status",
    "notifications.list",
    "notifications.create",
    "notifications.clear",
];
const PERMISSIONS: &[&str] = &[
    "nativeMessaging",
    "storage",
    "tabs",
    "webNavigation",
    "scripting",
    "debugger",
    "tabGroups",
    "contextMenus",
    "bookmarks",
    "history",
    "downloads",
    "cookies",
    "declarativeNetRequestWithHostAccess",
    "clipboardRead",
    "clipboardWrite",
    "offscreen",
    "alarms",
    "sessions",
    "downloads.ui",
    "sidePanel",
    "pageCapture",
    "search",
    "power",
    "idle",
    "webRequest",
    "management",
    "notifications",
];

pub(super) fn schema(action: &str) -> Value {
    let mut properties = json!({"tabId":{"type":"integer","minimum":0,"maximum":2_147_483_647}});
    let mut required = vec!["tabId"];
    if matches!(action, "notifications.create" | "notifications.clear") {
        properties["id"] = json!({"type":"string","pattern":"^[a-zA-Z0-9_.-]{1,80}$"});
        required.push("id");
    }
    if action == "notifications.create" {
        properties["title"] = json!({"type":"string","maxLength":128});
        properties["message"] = json!({"type":"string","maxLength":4096});
        required.extend(["title", "message"]);
    }
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
}

pub(super) fn steps(action: &str, args: &Value) -> Result<Value, String> {
    let schema = schema(action);
    if args.as_object().is_none_or(|fields| {
        fields
            .keys()
            .any(|key| schema["properties"].get(key).is_none())
    }) {
        return Err("Not run: unknown extension action argument; use chrome_status for this action's schema.".into());
    }
    let mut steps = Vec::new();
    let method = match action {
        "extensions.list" => "management.getAll",
        "extensions.status" => "permissions.status",
        "notifications.list" | "notifications.create" | "notifications.clear" => {
            "notifications.getAll"
        }
        _ => {
            return Err(
                "Unknown extension action; use chrome_status to find supported actions.".into(),
            );
        }
    };
    if matches!(action, "notifications.create" | "notifications.clear") {
        let id = args["id"]
            .as_str()
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 80
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            })
            .ok_or(
                "A notification needs a short ID of letters, digits, underscore, dot or hyphen.",
            )?;
        let mut params = json!({"id":id});
        if action == "notifications.create" {
            for (key, cap) in [("title", 128), ("message", 4096)] {
                let text=args[key].as_str().filter(|text| text.encode_utf16().count()<=cap).ok_or("Notification title/message is missing or too long; shorten it before trying again.")?;
                params[key] = json!(text);
            }
        }
        steps.push(json!({"id":"change","op":"api","method":action,"params":params}));
    }
    steps.push(json!({"id":"state","op":"api","method":method}));
    Ok(json!(steps))
}

pub(super) fn readback(action: &str, args: &Value, results: &Value) -> Result<Value, String> {
    let state = &results["state"];
    if action == "extensions.status" {
        let granted=state["granted"].as_array().ok_or("The extension did not return its granted permissions; check chrome_status before trying again.")?;
        let missing: Vec<_> = PERMISSIONS
            .iter()
            .filter(|permission| !granted.iter().any(|g| g == **permission))
            .collect();
        return Ok(
            json!({"permissionState":if missing.is_empty(){"granted"}else{"missing_permissions"},"missingPermissions":missing,"fileAccess":state["fileAccess"],"fileAccessInstruction":if state["fileAccess"]==true{Value::Null}else{json!("File access is off. Ask the user to enable Allow access to file URLs in this extension's browser details, then retry the file action.")}}),
        );
    }
    if action == "extensions.list" && !state.is_array() {
        return Err(
            "Extension inventory is unavailable; check chrome_status before trying again.".into(),
        );
    }
    if action.starts_with("notifications.") {
        let ids=state["ids"].as_array().ok_or("The notification inventory was not returned; check notifications.list before trying again.")?;
        if matches!(action, "notifications.create" | "notifications.clear") {
            let present = ids.contains(&args["id"]);
            if present != (action == "notifications.create") {
                return Err("The requested notification state was not confirmed; check notifications.list before trying again.".into());
            }
            return Ok(
                json!({"id":args["id"],"title":args["title"],"message":args["message"],"present":present}),
            );
        }
    }
    Ok(state.clone())
}

pub(super) async fn send(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<owner::Answer, String> {
    let action = request["action"].as_str().unwrap_or_default();
    let args = &request["args"];
    let mut result =
        page_plan::dispatch(shared, chat, request, consent, steps(action, args)?).await?;
    if result["status"] == "completed" {
        result["data"] = readback(action, args, &result["data"]["results"])?;
    }
    if action == "extensions.status" && result["status"] == "completed" {
        let addon =
            super::agent_browser::selected(shared, chat, "chrome_execute", request).addon(shared);
        result["data"]["extension"] = json!({"state":addon.state,"version":addon.extension_version,"instruction":addon.instruction});
    }
    Ok(page_plan::mcp(&result))
}
