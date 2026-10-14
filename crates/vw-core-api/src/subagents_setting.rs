//! Saves the Subagents setting for new chats (`sessionTools {op: "setSubagents"}`).
//!
//! Vibe 2.26.1 refuses to change `enable_subagents` through a chat that already exists ("settings.tools.subagents
//! cannot be reconfigured after session creation"). The value is saved anyway, but the refusal answers the write with
//! an error and leaves that chat unusable: every later turn in it fails the same way, also after reopening it. The
//! refusal does not apply to a chat that has run no turn, and `config/write` needs some chat to go through. So the
//! setting is written through a short-lived empty chat that this module starts, uses and deletes again; the user's
//! own chats are never touched. Chats that are already open keep the value they started with; new chats get the
//! saved one.
//!
//! The short-lived chat:
//! - opens in the Vibe server's own working folder (`Supervisor::work_dir`, the app's `work/` folder), never in a
//!   user's project folder, and runs no turn, so the agent writes nothing there or anywhere else;
//! - is recorded as a temporary session before anything is written through it, so a crash between the start and the
//!   delete still ends it at the next start (`temporary.rs`);
//! - is deleted without a dialog ([`Core::discard_unused`]); a failed delete is tried once more, and when it fails
//!   again the answer names the chat (`leftover_session_id`): it stays in the list as a temporary chat and goes away
//!   when Vibe Work quits.
//!
//! Depends on: `temporary` (record, `discard_unused`), `dispatch` (`session_call`, `forward`: the gate checks the
//! `config/write`), `vw-supervisor` (`work_dir`). Called by: `session_tools`.
//! Invariants:
//! - The write is exactly `[{op: "set", path: "/enable_subagents", value}]`, so the gate's allowlist applies as for
//!   any other `config/write`; nothing else can be written through this op.
//! - The short-lived chat is deleted whether or not the write worked.
//! - A write Vibe refuses is an error to the caller (and the chat is still deleted); a value that was saved but could
//!   not be read back is `enabled: null`, not an error.

use std::time::Duration;

use serde_json::{Value, json};
use tracing::warn;

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::session_tools_view::SessionToolsResult;

/// The config path of the setting.
const PATH: &str = "/enable_subagents";
/// The pause before the one retry of a failed delete.
const RETRY_PAUSE: Duration = Duration::from_millis(500);

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
        self.shared.temporary.ensure_recordable()?;
        let work_dir = self.shared.supervisor.work_dir();
        let start = json!({"agentConfig": {"cwd": work_dir.to_string_lossy()}});
        let id = match self.session_call(origin, "session/start", start).await? {
            crate::CommandResult::Session { snapshot, .. } => snapshot.session_id,
            _ => {
                return Err(CoreError::new(
                    ErrorCode::Internal,
                    "session/start did not answer with a session",
                ));
            }
        };
        // Recorded before the write, so a crash from here on still deletes the chat at the next start.
        if let Err(why) = self.shared.temporary.add(&id) {
            let gone = self.discard_unused(&id).await;
            return Err(CoreError::new(
                ErrorCode::Internal,
                format!(
                    "Subagents were not changed: the record of the helper chat could not be saved ({why}).{}",
                    if gone {
                        ""
                    } else {
                        " The helper chat could not be removed and stays in the list."
                    }
                ),
            ));
        }
        let written = self.write_and_read(origin, &id, enabled).await;
        let gone = self.discard_with_retry(&id).await;
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

    /// Deletes the short-lived chat; a failed delete is tried once more after a short pause.
    async fn discard_with_retry(&self, id: &str) -> bool {
        if self.discard_unused(id).await {
            return true;
        }
        warn!(
            session_id = id,
            "subagents setting: the helper chat was not deleted; trying once more"
        );
        tokio::time::sleep(RETRY_PAUSE).await;
        self.discard_unused(id).await
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
