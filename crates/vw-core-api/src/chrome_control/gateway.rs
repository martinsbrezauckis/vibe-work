//! Session-authenticated Chrome tools. Depends on the existing Chrome guard and owned helper;
//! called only by agent tools. Discovery and every operation require this chat's first-use consent.
use super::{mapping, owner};
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::{Arc, PoisonError};

pub(crate) fn is_tool(name: &str) -> bool {
    owner::TOOLS.contains(&name)
}

pub(super) fn restricted(shared: &Shared, action: &str) -> Option<&'static str> {
    let cap = mapping::capability(action)?;
    match shared
        .chrome_control
        .lowered
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(cap)
        .map(String::as_str)
    {
        Some("never") => Some("never"),
        Some("ask") => Some("ask"),
        _ => None,
    }
}

pub(crate) fn list(shared: &Shared, session: &str) -> Value {
    if shared.store.view(session).is_none()
        || !shared
            .agent_tools
            .as_ref()
            .is_some_and(|tools| tools.on_demand.chrome_allowed(session))
    {
        return json!([]);
    }
    json!(
        shared
            .chrome_control
            .owner
            .tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    )
}

pub(crate) async fn call(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    if shared.store.view(session).is_none() {
        return Err("Not run: this chat is not open in Vibe Work.".to_owned());
    }
    if !shared
        .agent_tools
        .as_ref()
        .is_some_and(|tools| tools.on_demand.chrome_allowed(session))
    {
        return Err("Not run: use_your_chrome must be allowed in this chat first.".to_owned());
    }
    if let Some(task) = args["taskId"].as_str() {
        if shared
            .chrome_control
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(task)
            .map(String::as_str)
            != Some(session)
        {
            return Err(
                "Not run: this Chrome task belongs to another chat or an earlier helper launch."
                    .to_owned(),
            );
        }
    } else if tool == "chrome_execute" {
        return Err("Not run: request a Chrome task in this chat first.".to_owned());
    }
    // Restrictions may have changed while the card waited.
    if restricted(shared, &mapping::action(tool, &args)) == Some("never") {
        return Err("Not run: this Chrome action was turned off in Chrome.".to_owned());
    }
    if shared.store.view(session).is_none() {
        return Err("Not run: this chat closed.".to_owned());
    }
    let request = tool == "chrome_task" && args["action"] == "request";
    let mut result = shared.chrome_control.owner.call(tool, args).await;
    let mut tasks = shared
        .chrome_control
        .tasks
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if request && let Some(id) = result["structuredContent"]["taskId"].as_str() {
        tasks.insert(id.to_owned(), session.to_owned());
    }
    if tool == "chrome_status" && result["structuredContent"].get("profiles").is_some() {
        for key in ["tasks", "pending"] {
            if let Some(rows) = result["structuredContent"][key].as_array_mut() {
                rows.retain(|row| {
                    row["taskId"]
                        .as_str()
                        .is_some_and(|id| tasks.get(id).map(String::as_str) == Some(session))
                });
            }
        }
        result["content"] = json!([{"type":"text","text":serde_json::to_string(&result["structuredContent"]).unwrap_or_default()}]);
    }
    drop(tasks);
    if let Some(text) = result["content"]
        .as_array()
        .and_then(|content| content.iter().find_map(|c| c["text"].as_str()))
        && (text.starts_with("Not run:") || text.starts_with("Outcome unknown:"))
    {
        shared
            .chrome_control
            .not_done(session, "Chrome action", text, shared.asks.now_ms());
    }
    Ok(json!({"vwMcpResult":result}))
}
