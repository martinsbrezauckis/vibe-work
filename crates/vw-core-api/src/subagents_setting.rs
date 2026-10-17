//! Saves the Subagents setting for new chats (`sessionTools {op: "setSubagents"}`).
//!
//! Vibe applies the Subagents setting (`enable_subagents`) when a chat is created, and `config/write` goes through
//! a chat. So the setting is written through a short-lived empty chat that this module starts, uses and deletes
//! again; the user's own chats are never touched. Chats that are already open keep the value they started with; new
//! chats get the saved one.
//!
//! The short-lived chat is the shared `helper_chat`: it opens in the Vibe server's own working folder, runs no turn,
//! is recorded as temporary before anything is written through it and is deleted without a dialog; a failed delete is
//! tried once more, and when it fails again the answer names the chat (`leftover_session_id`).
//!
//! Depends on: `helper_chat` (start, delete), `dispatch` (`forward`: the gate checks the `config/write`). Called by:
//! `session_tools`.
//! Invariants:
//! - The write is exactly `[{op: "set", path: "/enable_subagents", value}]`, so the gate's allowlist applies as for
//!   any other `config/write`; nothing else can be written through this op.
//! - The short-lived chat is deleted whether or not the write worked.
//! - A write Vibe refuses is an error to the caller (and the chat is still deleted); a value that was saved but could
//!   not be read back is `enabled: null`, not an error.

use serde_json::{Value, json};
use tracing::warn;

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::session_tools_view::SessionToolsResult;

/// The config path of the setting.
const PATH: &str = "/enable_subagents";

/// The value `config/fields/read` reports for the setting, when it reports a boolean.
fn reported_value(fields: &Value) -> Option<bool> {
    fields
        .get("fields")?
        .as_array()?
        .iter()
        .find(|field| field.get("name").and_then(Value::as_str) == Some("enable_subagents"))?
        .get("value")?
        .as_bool()
}

impl Core {
    /// Saves `enabled` as the Subagents setting through a short-lived empty chat (see the module header).
    ///
    /// # Errors
    /// `internal` when the temporary-session record is unusable or the chat cannot be started; Vibe's own error
    /// when it refuses the write (the chat is still deleted, and the message says so when that failed too).
    pub(crate) async fn set_subagents(
        &self,
        origin: Origin,
        enabled: bool,
    ) -> Result<SessionToolsResult, CoreError> {
        let id = self
            .start_helper_chat(origin, "Subagents were not changed")
            .await?;
        let written = self.write_and_read(origin, &id, enabled).await;
        let gone = self.end_helper_chat(&id).await;
        match (written, gone) {
            (Ok(saved), true) => Ok(SessionToolsResult::SubagentsSet {
                enabled: saved,
                leftover_session_id: None,
            }),
            (Ok(saved), false) => Ok(SessionToolsResult::SubagentsSet {
                enabled: saved,
                leftover_session_id: Some(id),
            }),
            (Err(error), true) => Err(error),
            (Err(mut error), false) => {
                error.message.push_str(
                    " The helper chat used for it could not be removed and stays in the list until Vibe Work closes.",
                );
                Err(error)
            }
        }
    }

    /// `config/write` through the short-lived chat, then the value Vibe reports back through it.
    async fn write_and_read(
        &self,
        origin: Origin,
        id: &str,
        enabled: bool,
    ) -> Result<Option<bool>, CoreError> {
        let write = json!({
            "sessionId": id,
            "ops": [{"op": "set", "path": PATH, "value": enabled}],
            "reason": "subagents for new chats",
            "reloadRuntime": true,
        });
        let reply = self.forward(origin, "config/write", write, None).await?;
        // Vibe can answer a write with a body that says it did not take it.
        if reply.result.get("rejected").and_then(Value::as_bool) == Some(true) {
            return Err(
                CoreError::new(ErrorCode::ServerError, "Vibe did not accept the change.")
                    .with_method("config/write"),
            );
        }
        if reply
            .result
            .get("failures")
            .and_then(Value::as_array)
            .is_some_and(|failures| !failures.is_empty())
        {
            return Err(
                CoreError::new(ErrorCode::ServerError, "Vibe could not save the change.")
                    .with_method("config/write"),
            );
        }
        // The value a new chat starts with. The write is done, so a failed read only means "not known here".
        match self
            .forward(origin, "config/fields/read", json!({"sessionId": id}), None)
            .await
        {
            Ok(read) => Ok(reported_value(&read.result)),
            Err(error) => {
                warn!(%error, "subagents setting: the saved value could not be read back");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::reported_value;
    use serde_json::json;

    #[test]
    fn reads_the_boolean_of_the_setting_and_nothing_else() {
        let fields = json!({"fields": [
            {"name": "enable_background_processes", "value": true},
            {"name": "enable_subagents", "value": false, "layerValues": []},
        ]});
        assert_eq!(reported_value(&fields), Some(false));
        let other = json!({"fields": [{"name": "enable_background_processes", "value": true}]});
        assert_eq!(reported_value(&other), None);
        let text = json!({"fields": [{"name": "enable_subagents", "value": "yes"}]});
        assert_eq!(reported_value(&text), None);
        assert_eq!(reported_value(&json!({"nope": 1})), None);
    }
}
