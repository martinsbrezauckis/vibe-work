//! A short-lived empty chat for settings that Vibe reads and writes only through a session
//! (`config/write`, `config/proxy/read|write`) when the caller has no suitable session of its own.
//!
//! The chat:
//! - opens in the Vibe server's own working folder (`Supervisor::work_dir`, the app's `work/` folder), never in a
//!   user's project folder, and runs no turn, so the agent writes nothing there or anywhere else;
//! - is recorded as a temporary session before anything is done through it, so a crash between the start and the
//!   delete still ends it at the next start (`temporary.rs`);
//! - is deleted without a dialog ([`Core::discard_unused`]); a failed delete is tried once more, and when it fails
//!   again the caller names the chat (`leftover_session_id`): it stays in the list as a temporary chat and goes
//!   away when Vibe Work quits.
//!
//! Depends on: `temporary` (record, `discard_unused`), `dispatch` (`session_call`), `vw-supervisor` (`work_dir`).
//! Called by: `subagents_setting` (the Subagents switch), `network` (Settings > Network).
//! Invariants: the user's own chats are never touched; every started chat is deleted by its caller through
//! [`Core::end_helper_chat`] whether or not the work succeeded.

use std::time::Duration;

use serde_json::json;
use tracing::warn;

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};

/// The pause before the one retry of a failed delete.
const RETRY_PAUSE: Duration = Duration::from_millis(500);

impl Core {
    /// Starts a short-lived empty chat and records it as temporary. `unchanged` finishes the sentence that tells
    /// the user nothing was done when the chat cannot be recorded (for example "Subagents were not changed").
    ///
    /// # Errors
    /// `internal` when the temporary-session record is unusable or the chat cannot be started; Vibe's own error for
    /// `session/start`.
    pub(crate) async fn start_helper_chat(
        &self,
        origin: Origin,
        unchanged: &str,
    ) -> Result<String, CoreError> {
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
        // Recorded before anything is done through it, so a crash from here on still deletes the chat at the next
        // start.
        if let Err(why) = self.shared.temporary.add(&id) {
            let gone = self.discard_unused(&id).await;
            return Err(CoreError::new(
                ErrorCode::Internal,
                format!(
                    "{unchanged}: the record of the helper chat could not be saved ({why}).{}",
                    if gone {
                        ""
                    } else {
                        " The helper chat could not be removed and stays in the list."
                    }
                ),
            ));
        }
        Ok(id)
    }

    /// Deletes the short-lived chat; a failed delete is tried once more after a short pause. True when it is gone.
    pub(crate) async fn end_helper_chat(&self, id: &str) -> bool {
        if self.discard_unused(id).await {
            return true;
        }
        warn!(
            session_id = id,
            "helper chat: not deleted; trying once more"
        );
        tokio::time::sleep(RETRY_PAUSE).await;
        self.discard_unused(id).await
    }
}
