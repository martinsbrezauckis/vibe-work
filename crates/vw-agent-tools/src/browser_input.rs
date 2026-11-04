//! Built-in browser input keys checked against the shipped schema, before consent or tab selection.
//! Used by the MCP server and core tool boundary; keeps extra argument objects out of execution.
use serde_json::Value;

/// Reject fields the built-in read/action executors do not advertise.
///
/// # Errors
/// The input is not an object or contains an unknown field.
pub fn validate(tool: &str, args: &Value) -> Result<(), String> {
    if !matches!(tool, "browser_read" | "browser_act") {
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
