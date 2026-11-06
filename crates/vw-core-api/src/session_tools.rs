//! The `sessionTools` command: routes each op to its module.
//!
//! Depends on: `session_export`, `session_excerpt`, `temporary` (the record, a folder-less chat's scratch folder),
//! `subagents_setting`, `session_tools_view`. Called by: `dispatch`.

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
    /// Makes a temporary chat's scratch folder and names it as `config`'s `cwd`. The core made the folder for this
    /// chat, so it counts as chosen (as a folder from the native dialog): the chat's views read it like any chat's.
    ///
    /// # Errors
    /// `internal` when the folder cannot be made; nothing was started.
    async fn scratch_folder(
        &self,
        config: &mut serde_json::Value,
    ) -> Result<std::path::PathBuf, CoreError> {
        let dir = self
            .shared
            .temporary
            .new_scratch_saved()
            .await
            .map_err(|why| {
                CoreError::new(
                    crate::ErrorCode::Internal,
                    format!("The temporary chat was not started: {why}. Nothing was created."),
                )
            })?;
        if let Err(error) = self.shared.gate.register_picked_folder(&dir) {
            let record = self.shared.temporary.clone();
            let _ = tokio::task::spawn_blocking(move || record.remove_scratch(&dir)).await;
            return Err(CoreError::from_gate("sessionTools", &error));
        }
        if !config.is_object() {
            *config = serde_json::json!({});
        }
        config["cwd"] = serde_json::Value::String(dir.to_string_lossy().into_owned());
        Ok(dir)
    }

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
                let mut config = agent_config.unwrap_or_else(|| serde_json::json!({}));
                // Without a folder the chat gets a scratch folder of its own (it goes with the chat).
                let scratch = if config["cwd"].as_str().is_some_and(|cwd| !cwd.is_empty()) {
                    None
                } else {
                    Some(self.scratch_folder(&mut config).await?)
                };
                let started = self
                    .session_call(
                        origin,
                        "session/start",
                        serde_json::json!({"agentConfig": config}),
                    )
                    .await;
                let started = match (started, &scratch) {
                    (Err(error), Some(dir)) => {
                        let (record, dir) = (self.shared.temporary.clone(), dir.clone());
                        let _ =
                            tokio::task::spawn_blocking(move || record.remove_scratch(&dir)).await;
                        return Err(error);
                    }
                    (started, _) => started?,
                };
                match started {
                    crate::CommandResult::Session { snapshot, .. } => {
                        let session_id = snapshot.session_id;
                        if let Some(dir) = scratch {
                            self.shared.temporary.bind_scratch(&session_id, dir);
                        }
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
