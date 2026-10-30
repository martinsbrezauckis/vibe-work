//! The `sessionTools` command: routes each op to its module.
//!
//! Depends on: `session_export`, `session_excerpt`, `temporary`, `subagents_setting`, `session_tools_view`. Called by: `dispatch`.

use crate::command::Origin;
use crate::core::Core;
use crate::error::CoreError;
use crate::session_tools_view::{SessionToolsRequest, SessionToolsResult};

fn other_kind(result: &crate::CommandResult) -> &'static str {
    match result {
        crate::CommandResult::Reply { .. } => "a reply",
        _ => "something else",
    }
}

impl Core {
    /// Executes one `sessionTools` request.
    ///
    /// # Errors
    /// As the op's handler: `export` fails when Vibe cannot read the session or the file cannot be written;
    /// `startTemporary` fails (and ends the new session again) when the temporary-session record cannot be written;
    /// `setSubagents` fails when Vibe does not take the value (its short-lived chat is still deleted).
    pub(crate) async fn session_tools(
        &self,
        origin: Origin,
        request: SessionToolsRequest,
    ) -> Result<SessionToolsResult, CoreError> {
        match request {
            SessionToolsRequest::Export {
                session_id,
                destination,
            } => Ok(SessionToolsResult::Exported {
                export: self
                    .export_session(origin, &session_id, destination)
                    .await?,
            }),
            SessionToolsRequest::StartTemporary { agent_config } => {
                self.shared.temporary.ensure_recordable()?;
                let params = match agent_config {
                    Some(config) => serde_json::json!({"agentConfig": config}),
                    None => serde_json::json!({}),
                };
                match self.session_call(origin, "session/start", params).await? {
                    crate::CommandResult::Session { snapshot, .. } => {
                        let session_id = snapshot.session_id;
                        // The record must be durable before the start is answered: a window told "started" sends
                        // its first message, and a session missing from the record would then survive a crash as
                        // an ordinary, kept chat. When the record cannot be written the session is ended again.
                        if let Err(why) = self.shared.temporary.add_saved(&session_id).await {
                            return Err(self.refuse_unrecorded(&session_id, &why).await);
                        }
                        Ok(SessionToolsResult::Started { session_id })
                    }
                    other => Err(CoreError::new(
                        crate::ErrorCode::Internal,
                        format!("session/start answered {}", other_kind(&other)),
                    )),
                }
            }
            SessionToolsRequest::Holder { session_id } => Ok(SessionToolsResult::Holder {
                holder_pid: self.session_holder(&session_id).await?,
            }),
            SessionToolsRequest::Temporary {} => Ok(SessionToolsResult::Temporary {
                session_ids: self.shared.temporary.ids(),
            }),
            SessionToolsRequest::SetSubagents { enabled } => {
                self.set_subagents(origin, enabled).await
            }
            SessionToolsRequest::Excerpt { session_id } => Ok(SessionToolsResult::Excerpt {
                excerpt: self.session_excerpt(&session_id).await?,
            }),
        }
    }
}
