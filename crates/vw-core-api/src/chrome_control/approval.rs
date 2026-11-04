//! Browser identity on approval entries, resolved from the action's helper session, never the chat display row.
//! Called at callback/history/snapshot boundaries and app-card creation; leaves tool input unchanged.
use super::agent_browser::Browser;
use crate::core::Shared;
use serde_json::Value;

pub(crate) fn project(shared: &Shared, value: &mut Value) {
    let effect = &value["detail"]["effect"];
    let tool = effect["toolName"].as_str().unwrap_or_default();
    if matches!(
        tool,
        "mcp_vw.chrome_execute"
            | "mcp_vw.chrome_session"
            | "mcp_vw.chrome_task"
            | "mcp_vw.chrome_status"
    ) {
        let input = &effect["input"];
        let browser = super::receipt::profile(shared, tool.trim_start_matches("mcp_vw."), input)
            .as_deref()
            .map(Browser::from_profile);
        value["vwBrowser"] =
            browser.map_or(Value::Null, |browser| Value::String(browser.name().into()));
        return;
    }
    // Only protocol containers carry entries; never interpret an action's nested arguments as callbacks.
    match value {
        Value::Array(items) => items.iter_mut().for_each(|item| project(shared, item)),
        Value::Object(map) => {
            for key in [
                "callback",
                "entry",
                "entries",
                "items",
                "history",
                "activeCallbacks",
                "state",
            ] {
                if let Some(child) = map.get_mut(key) {
                    project(shared, child);
                }
            }
        }
        _ => {}
    }
}
