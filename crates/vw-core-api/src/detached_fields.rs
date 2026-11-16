//! Vibe's settings for new chats without a chat of the caller's: `config/fields/read` sent without a `sessionId`.
//!
//! Vibe answers `config/fields/read` only through an attached session. After Vibe Work starts, no listed chat has a
//! channel, and a read through one of them is answered inertly (`detached_reads`: an empty list, no session opened),
//! so Settings asks for the settings themselves: the core reads them through the shared short-lived helper chat
//! (`helper_chat`: the Vibe server's own working folder, no turn, recorded as temporary, deleted again). The answer
//! is Vibe's own field list with every layer (defaults, the user's file, the environment, the administrator), so
//! Settings shows the stored values and every row. A chat's own session overrides and its project folder's settings
//! are not part of it: an open chat answers those itself. `account/read` and `identity/read` without a `sessionId`
//! are read the same way, so Settings › Account works before any chat exists.
//!
//! Depends on: `helper_chat` (start, delete), the gate (the helper's read passes it as any read). Called by:
//! `dispatch` through [`Core::answered_by_core`] (a `Call` of one of these reads without `sessionId`, from the
//! window or the Debug API), which also routes `tools/list` to `tools_list`.
//! Invariants: a read that names a chat never comes here (it goes to Vibe, or inertly to `detached_reads`); the
//! person's chats are never opened; every helper chat started here is deleted whether or not the read worked (a
//! failed delete leaves it in the list as a temporary chat until Vibe Work quits).

use serde_json::json;
use tracing::warn;

use crate::command::{Command, CommandResult, Origin};
use crate::core::Core;
use crate::error::CoreError;

/// The reads answered here when they name no chat: Vibe's settings for new chats, and the account (Settings ›
/// Account on a fresh install, before any chat exists).
const WITHOUT_CHAT: [&str; 3] = ["config/fields/read", "account/read", "identity/read"];

impl Core {
    /// The answer to a `Call` the core gives itself, else `None` (the call goes on to Vibe): `tools/list`, which Vibe's
    /// unified harness lacks (answered from the session's `runtime/read`), and `config/fields/read` without a chat.
    pub(crate) async fn answered_by_core(
        &self,
        origin: Origin,
        command: &Command,
    ) -> Option<Result<CommandResult, CoreError>> {
        let Command::Call { method, params, .. } = command else {
            return None;
        };
        if method == crate::tools_list::METHOD {
            return Some(crate::tools_list::answer(self, origin, params.clone()).await);
        }
        if WITHOUT_CHAT.contains(&method.as_str()) && params.get("sessionId").is_none() {
            return Some(self.read_without_chat(origin, method).await);
        }
        None
    }

    /// `method` through a short-lived helper chat (see the module header).
    async fn read_without_chat(
        &self,
        origin: Origin,
        method: &str,
    ) -> Result<CommandResult, CoreError> {
        let helper = self
            .start_helper_chat(origin, "Settings could not be read")
            .await?;
        let reply = self
            .shared
            .gate
            .call(method, json!({"sessionId": helper}), None)
            .await;
        if !self.end_helper_chat(&helper).await {
            warn!(
                session_id = helper,
                "settings read: the helper chat could not be deleted"
            );
        }
        let reply = reply.map_err(|e| CoreError::from_gate(method, &e))?;
        Ok(CommandResult::Reply {
            result: reply.result,
            client_user_message_id: None,
            user_entry_ids: Vec::new(),
        })
    }
}
