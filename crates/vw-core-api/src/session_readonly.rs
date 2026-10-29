//! Mutation admission for foreign sessions, before dialogs, callbacks or local side effects.
//! Depends on the typed command and lease holder; execute holds a temporary claim through the command.
//! Unknown session operations default to mutations; reading a held session never acquires its lease.

use serde_json::Value;
use std::sync::Arc;

use crate::command::Command;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::session_lease::Lease;

fn readonly(command: &Command, wire: &Value) -> bool {
    match command {
        Command::Files { request } => {
            !matches!(request, crate::files_view::FilesRequest::Write { .. })
        }
        Command::ReadSession { .. }
        | Command::RefreshSession { .. }
        | Command::ViewSession { .. } => true,
        Command::Call { method, .. } => {
            method.ends_with("/read") || method.ends_with("/list") || method.ends_with("/subscribe")
        }
        _ => matches!(
            wire.pointer("/request/op").and_then(Value::as_str),
            Some(
                "read"
                    | "list"
                    | "holder"
                    | "export"
                    | "inspect"
                    | "status"
                    | "overview"
                    | "readFile"
                    | "diff"
                    | "snapshot"
                    | "tree"
                    | "branches"
                    | "log"
            )
        ),
    }
}

impl Core {
    pub(crate) async fn session_change_lease(
        &self,
        command: &Command,
    ) -> Result<Option<Arc<Lease>>, CoreError> {
        let wire = serde_json::to_value(command)
            .map_err(|error| CoreError::new(ErrorCode::Internal, error.to_string()))?;
        if readonly(command, &wire) {
            return Ok(None);
        }
        let scope = wire
            .get("params")
            .or_else(|| wire.get("request"))
            .unwrap_or(&wire);
        let terminal = scope
            .get("terminalId")
            .and_then(Value::as_str)
            .and_then(|id| {
                self.shared
                    .terminals
                    .info(&vw_terminal::TerminalId::new(id.to_owned()))
                    .ok()
            })
            .map(|info| info.session_id);
        let Some(id) = scope
            .get("sessionId")
            .or_else(|| scope.get("sourceSessionId"))
            .and_then(Value::as_str)
            .or(terminal.as_deref())
        else {
            return Ok(None);
        };
        if id.is_empty() {
            return Ok(None);
        }
        // Preview requests also accept a page key in sessionId. Ownership belongs to its chat.
        let preview_owner = if matches!(command, Command::Preview { .. }) {
            self.shared
                .preview
                .as_ref()
                .and_then(|hub| hub.service.owner_session_id(id))
        } else {
            None
        };
        let id = preview_owner.as_deref().unwrap_or(id);
        // The Settings sign-in page is no Vibe session; only its own account commands reach it.
        if id == crate::browser_accounts::ACCOUNT_PAGE {
            return Err(crate::browser_accounts::user_only());
        }
        // The New chat screen's owner and a folder's own key (`files-workspace:<folder>`: the Files tab and its Commit
        // dialog before any chat) are not Vibe sessions: there is no lease to hold and nothing to ask Vibe.
        // Vibe's `session/read` refuses these folder keys as invalid session ids.
        if crate::new_chat::is_draft(&self.shared, id) || id.starts_with(crate::explorer::WORKSPACE)
        {
            return Ok(None);
        }
        let deleting = matches!(command,
            Command::Call { method, .. } | Command::RequestConfirmation { method, .. }
                if method == "session/delete");
        // A damaged store must still reach its Delete dialog and Vibe's own delete. Keep the app lease;
        // Vibe remains responsible for refusing a genuinely held session. Other mutations still fail closed.
        let holder = match self.session_holder(id).await {
            Err(error) if deleting && error.server_code.as_deref() == Some("internal_error") => {
                None
            }
            result => result?,
        };
        if let Some(pid) = holder {
            return Err(crate::session_holder::held_refusal(pid));
        }
        self.claim_session(id).await
    }
}
