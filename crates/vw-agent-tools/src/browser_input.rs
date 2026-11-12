//! Built-in browser input keys checked against the shipped schema, before consent or tab selection.
//! Used by the MCP server and core tool boundary; keeps extra argument objects out of execution.
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};

/// The executable action names and their fixed approval verbs, shared with the UI.
/// A malformed catalog offers no actions or session scope.
#[must_use]
pub fn actions() -> &'static BTreeMap<String, String> {
    static ACTIONS: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        serde_json::from_str::<Value>(include_str!("browser_actions.json"))
            .ok()
            .and_then(|value| serde_json::from_value(value["actions"].clone()).ok())
            .unwrap_or_default()
    })
}

/// Reject fields the built-in read/action executors do not advertise.
///
/// # Errors
/// The input is not an object or contains an unknown field.
pub fn validate(tool: &str, args: &Value) -> Result<(), String> {
    if !matches!(tool, "browser_read" | "browser_act") {
        return Ok(());
    }
    if tool == "browser_act" && args.get("steps").is_some() {
        let fields = args
            .as_object()
            .ok_or("Batch arguments must be an object.")?;
        if fields.len() != 1 {
            return Err(
                "A batch takes only steps; put each action's fields inside its step.".into(),
            );
        }
        let steps = args["steps"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 16)
            .ok_or("A batch needs 1 to 16 steps; split larger work into separate calls.")?;
        for step in steps {
            if step.get("steps").is_some()
                || !step["action"]
                    .as_str()
                    .is_some_and(|a| actions().contains_key(a))
            {
                return Err(
                    "Each batch step needs one advertised action; nested batches are unavailable."
                        .into(),
                );
            }
            validate(tool, step)?;
        }
        return Ok(());
    }
    let fields = args
        .as_object()
        .ok_or("browser arguments must be an object")?;
    let tools = crate::specs::tool_list(false);
    let properties = tools
        .as_array()
        .and_then(|tools| tools.iter().find(|spec| spec["name"] == tool))
        .and_then(|spec| spec["inputSchema"]["properties"].as_object())
        .ok_or("browser input schema unavailable")?;
    if let Some(key) = fields.keys().find(|key| !properties.contains_key(*key)) {
        return Err(format!(
            "unknown {tool} field: {key}; use the advertised top-level fields"
        ));
    }
    Ok(())
}
