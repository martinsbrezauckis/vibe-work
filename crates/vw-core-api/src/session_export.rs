//! "Export as Markdown" for any session.
//!
//! The whole history is read with `session/history/list`, newest page first, following `nextCursor` to older
//! pages (`direction: "backward"`, items oldest-first within a page,
//! `nextCursor` = the next OLDER page, `null` at the start). For a session no channel holds the call goes to the
//! lobby, which reads the stored session without taking it (`server.py:834-843`), so exporting never takes a
//! session from the Vibe CLI. The text is rendered by `export_md`, then every known secret value is replaced
//! (`vw_secrets::scrub`, the same registry the Debug API uses).
//!
//! Where it goes: the window through the native save dialog (the app's `export_session` command calls
//! [`Core::export_session_to`]; the webview never names a path); the Debug API (acting as the user)
//! names `destination`, or gets the text back without one.
//!
//! Depends on: `export_md`, `save_file`, vw-secrets, the gate. Called by: `dispatch` (`exportSession`), vw-host
//! (the save dialog's command).
//! Invariants: read-only towards Vibe (only `session/history/list` and `session/read`); at most
//! [`MAX_PAGES`] pages of [`PAGE`] entries are read (a cut is said in the answer); a destination from the window is
//! refused.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::export_md::{ExportHead, render};
use crate::session_tools_view::SessionExportView;

/// Entries per history page (Vibe's maximum).
const PAGE: u64 = 500;
/// Most pages read (25,000 entries).
const MAX_PAGES: usize = 50;

/// A file name from a title: letters, digits, spaces, `-` and `_` kept, the rest dropped, at most 80 characters.
pub(crate) fn file_name_for(title: Option<&str>, session_id: &str) -> String {
    let cleaned: String = title
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(80)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        let short: String = session_id.chars().take(8).collect();
        format!("Vibe session {short}.md")
    } else {
        format!("{trimmed}.md")
    }
}

fn text_at<'a>(state: &'a Value, pointer: &str) -> Option<&'a str> {
    state.pointer(pointer).and_then(Value::as_str)
}

impl Core {
    /// The session's public state: the store's copy when loaded, else a `session/read` with one-entry pages.
    async fn export_state(&self, session_id: &str) -> Result<Value, CoreError> {
        if let Some(view) = self.shared.store.view(session_id) {
            return Ok(view.state);
        }
        let page = json!({"limit": 1});
        let params = json!({"sessionId": session_id, "history": page, "turns": page});
        let reply = self
            .shared
            .gate
            .call("session/read", params, None)
            .await
            .map_err(|e| CoreError::from_gate("session/read", &e))?;
        Ok(reply.result.get("state").cloned().unwrap_or(Value::Null))
    }

    /// Every history entry of the session, oldest first, and whether the read stopped at [`MAX_PAGES`].
    pub(crate) async fn full_history(
        &self,
        session_id: &str,
    ) -> Result<(Vec<Value>, bool), CoreError> {
        let mut pages: Vec<Vec<Value>> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut page = json!({"limit": PAGE, "direction": "backward"});
            if let Some(at) = &cursor {
                page["cursor"] = json!(at);
            }
            let reply = self
                .shared
                .gate
                .call(
                    "session/history/list",
                    json!({"sessionId": session_id, "page": page}),
                    None,
                )
                .await
                .map_err(|e| CoreError::from_gate("session/history/list", &e))?;
            let items = reply
                .result
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let empty = items.is_empty();
            pages.push(items);
            cursor = reply
                .result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() || empty {
                let entries = pages.into_iter().rev().flatten().collect();
                return Ok((entries, false));
            }
        }
        Ok((pages.into_iter().rev().flatten().collect(), true))
    }

    /// Builds the Markdown of a session (secret values replaced).
    async fn build_session_export(
        &self,
        session_id: &str,
    ) -> Result<(String, SessionExportView), CoreError> {
        let state = self.export_state(session_id).await?;
        let (entries, cut) = self.full_history(session_id).await?;
        let title = text_at(&state, "/session/title").map(str::to_owned);
        let head = ExportHead {
            title: title.clone(),
            cwd: text_at(&state, "/session/cwd").map(str::to_owned),
            model: text_at(&state, "/session/model").map(str::to_owned),
            session_id: session_id.to_owned(),
            exported_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0)),
        };
        let (mut markdown, counts) = render(&head, &entries);
        if cut {
            markdown.push_str(
                "\n*The oldest part of this session is not in this export (it is longer than Vibe Work reads).*\n",
            );
        }
        let mut secrets_replaced = Vec::new();
        if let Some((scrubbed, kinds)) = vw_secrets::scrub(markdown.as_bytes()) {
            markdown = String::from_utf8_lossy(&scrubbed).into_owned();
            secrets_replaced = kinds.iter().map(|k| k.as_str().to_owned()).collect();
        }
        let view = SessionExportView {
            name: file_name_for(title.as_deref(), session_id),
            bytes: markdown.len() as u64,
            messages: counts.messages,
            steps: counts.steps,
            diffs: counts.diffs,
            secrets_replaced,
            cut,
            saved_to: None,
            markdown: None,
        };
        Ok((markdown, view))
    }

    /// `exportSession` (see the module header).
    pub(crate) async fn export_session(
        &self,
        origin: Origin,
        session_id: &str,
        destination: Option<String>,
    ) -> Result<SessionExportView, CoreError> {
        if destination.is_some() && origin != Origin::DebugApi {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "the window saves an export through the native save dialog",
            ));
        }
        let (markdown, mut view) = self.build_session_export(session_id).await?;
        match destination {
            None => view.markdown = Some(markdown),
            Some(path) => {
                view.saved_to = Some(
                    crate::save_file::write_whole(markdown.into_bytes(), Path::new(&path)).await?,
                );
            }
        }
        Ok(view)
    }

    /// Exports `session_id` to `destination`, a file the user chose in the native save dialog (the app's command).
    ///
    /// # Errors
    /// The gate's error when Vibe cannot read the session; `invalidParams` for a bad destination; `internal` when
    /// the file cannot be written.
    pub async fn export_session_to(
        &self,
        session_id: &str,
        destination: &Path,
    ) -> Result<SessionExportView, CoreError> {
        let (markdown, mut view) = self.build_session_export(session_id).await?;
        view.saved_to =
            Some(crate::save_file::write_whole(markdown.into_bytes(), destination).await?);
        Ok(view)
    }

    /// The suggested file name for a session's export (the save dialog's default).
    pub async fn export_session_name(&self, session_id: &str) -> String {
        let title = self
            .export_state(session_id)
            .await
            .ok()
            .and_then(|state| text_at(&state, "/session/title").map(str::to_owned));
        file_name_for(title.as_deref(), session_id)
    }
}
