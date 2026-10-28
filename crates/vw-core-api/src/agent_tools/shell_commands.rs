//! Exact shell commands observed by our hook, before Vibe projects grep into a file search.
//! Only the hook credential records them; pump/snapshot projection attaches display-only metadata.
//! Bounded by count and command size; a missing command never becomes a reconstructed approval.
use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

#[derive(Default)]
pub(crate) struct Commands(Mutex<VecDeque<(String, String, String)>>);

impl Commands {
    pub(crate) fn record(&self, args: &Value) -> Result<Value, String> {
        let text = |key| {
            args.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        };
        let (Some(session), Some(call), Some(command)) =
            (text("sessionId"), text("toolCallId"), text("command"))
        else {
            return Err("missing shell command identity".into());
        };
        if command.len() > 32_768 || session.len() > 256 || call.len() > 256 {
            return Err("shell command identity is too long".into());
        }
        let mut items = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        // A call id is immutable. Reusing one with different text cannot change an open card.
        if let Some((_, _, kept)) = items.iter().find(|(s, c, _)| s == session && c == call) {
            return if kept == command {
                Ok(Value::Null)
            } else {
                Err("shell command identity changed".into())
            };
        }
        items.push_back((session.into(), call.into(), command.into()));
        while items.len() > 256 {
            items.pop_front();
        }
        Ok(Value::Null)
    }

    pub(crate) fn project(&self, value: &mut Value) {
        let kind = value["type"].as_str();
        if kind == Some("callback") {
            let session = value["sessionId"].as_str().unwrap_or_default();
            let id = value["callbackId"]
                .as_str()
                .or_else(|| value["id"].as_str())
                .unwrap_or_default();
            let call = id.strip_prefix("approval-").unwrap_or(id);
            let command = self
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .find(|(s, c, _)| s == session && c == call)
                .map(|(_, _, text)| text.clone());
            if let Some(detail) = value.pointer_mut("/detail/effect")
                && detail["kind"] == "file_search"
                && let Some(command) = command
            {
                detail["vwCommand"] = Value::String(command);
            }
        }
        match value {
            Value::Object(map) => map.values_mut().for_each(|v| self.project(v)),
            Value::Array(list) => list.iter_mut().for_each(|v| self.project(v)),
            _ => {}
        }
    }
}

pub(crate) fn project(shared: &crate::core::Shared, value: &mut Value) {
    if let Some(tools) = &shared.agent_tools {
        tools.shell_commands.project(value);
    }
}

/// A projected search approval must have the exact command observed by our authenticated hook.
pub(crate) fn verified(params: &Value) -> Result<(), vw_rpc::HandlerError> {
    let effect = &params["callback"]["detail"]["effect"];
    if effect["kind"] == "file_search" && effect["vwCommand"].as_str().is_none() {
        return Err(vw_rpc::HandlerError::new(
            vw_protocol::ProtocolErrorCode::InvalidParams,
            "Vibe Work could not verify the exact search command, so it was not approved.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Commands;
    use serde_json::json;

    #[test]
    fn observed_commands_belong_to_one_immutable_session_call() {
        let commands = Commands::default();
        let observed = json!({"sessionId":"s", "toolCallId":"c", "command":"grep -nH 'a' f"});
        commands.record(&observed).unwrap();
        commands.record(&observed).unwrap();
        let mut changed = observed.clone();
        changed["command"] = json!("different");
        assert!(commands.record(&changed).is_err());
        let mut card = json!({"type":"callback", "sessionId":"other", "id":"approval-c", "detail":{"effect":{"kind":"file_search"}}});
        commands.project(&mut card);
        assert!(card["detail"]["effect"]["vwCommand"].is_null());
        card["sessionId"] = json!("s");
        commands.project(&mut card);
        assert_eq!(card["detail"]["effect"]["vwCommand"], observed["command"]);
    }
}
