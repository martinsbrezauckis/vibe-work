//! Keeps Vibe's structured validation diagnosis in errors for both clients.
//! Dispatch and recovery add a repair location only when the app's config contains the rejected rule.
//! Reads bounded TOML; never includes config values or unrelated RPC error data in the message.
use serde_json::Value;
use vw_gate::GateError;
use vw_rpc::RpcError;

use crate::core::Shared;
use crate::error::CoreError;

fn issues(error: &vw_protocol::ProtocolError) -> Option<&Vec<Value>> {
    (error.code.as_str() == "invalid_params")
        .then(|| error.data.as_option()?.get("issues")?.as_array())?
}

fn location(issue: &Value) -> Option<String> {
    let path = issue["path"].as_array()?;
    let parts: Option<Vec<_>> = path
        .iter()
        .map(|part| match part {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .collect();
    Some(parts?.join("."))
}

/// The same field/message detail Vibe's own client renders, without dumping arbitrary error data.
pub(crate) fn detail(error: &vw_protocol::ProtocolError) -> Option<String> {
    let parts: Vec<_> = issues(error)?
        .iter()
        .filter_map(|issue| {
            let path = location(issue)?;
            let message = issue["message"].as_str()?;
            Some(if path.is_empty() {
                message.to_owned()
            } else {
                format!("{path}: {message}")
            })
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Adds the source and one repair instruction to a failed session open.
pub(crate) fn from_gate(shared: &Shared, method: &str, error: &GateError) -> CoreError {
    let mut failure = CoreError::from_gate(method, error);
    if !matches!(method, "session/start" | "session/resume") {
        return failure;
    }
    let GateError::Transport(RpcError::Remote { error, .. }) = error else {
        return failure;
    };
    let Some(issues) = issues(error) else {
        return failure;
    };
    let path = shared.instructions.own_home.join("config.toml");
    let config = crate::default_model::read(&path)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok());
    let Some(config) = config else {
        return failure;
    };
    let mut diagnoses = Vec::new();
    let mut found_source = false;
    for issue in issues {
        let Some(field) = location(issue) else {
            continue;
        };
        let Some(message) = issue["message"].as_str() else {
            continue;
        };
        let before = diagnoses.len();
        // Vibe validates each tool separately: its issue path loses the tool's prefix.
        // Only name a permission rule here when it actually violates Vibe's reported enum.
        if field == "permission"
            && message == "Input should be 'always', 'never' or 'ask'"
            && let Some(tools) = config.get("tools").and_then(toml::Value::as_table)
        {
            for (tool, settings) in tools {
                if let Some(permission) = settings.get("permission")
                    && !matches!(permission.as_str(), Some("always" | "never" | "ask"))
                {
                    diagnoses.push(format!("tools.{tool}.permission: {message}"));
                }
            }
        }
        if diagnoses.len() == before {
            diagnoses.push(format!("{field}: {message}"));
        } else {
            found_source = true;
        }
    }
    if found_source {
        failure.message = format!(
            "{}. Open {} to fix the rule.",
            crate::error::clip(&diagnoses.join("; "), 600),
            path.display()
        );
    }
    failure
}
