//! Share a page as one file and export a session as Markdown: the app's native save dialog
//! hands the chosen file here, and the core writes the export there. The webview only names the session and the page; it never supplies the destination.
//!
//! Depends on: vw-core-api (`Core::export_page_to`), `host`. Called by: `app` (`export_page` command).
//! Invariant: the destination comes from the OS save dialog (Rust side), never from a client request.

use std::path::Path;

use vw_core_api::{CoreError, PagesResult, SessionExportView};

use crate::host::Host;

impl Host {
    /// Writes `path` of `session_id`'s handbook as one file to `destination` (the save dialog's choice).
    ///
    /// # Errors
    /// `serverUnavailable` without a core; the core's errors (unknown session or page, a bad destination).
    pub async fn export_page(
        &self,
        session_id: &str,
        path: &str,
        destination: &Path,
    ) -> Result<PagesResult, CoreError> {
        self.core_or_unavailable()?
            .export_page_to(session_id, path, destination)
            .await
    }

    /// Writes `session_id` as Markdown to `destination` (the save dialog's choice).
    ///
    /// # Errors
    /// `serverUnavailable` without a core; the core's errors (Vibe cannot read the session, a bad destination).
    pub async fn export_session(
        &self,
        session_id: &str,
        destination: &Path,
    ) -> Result<SessionExportView, CoreError> {
        self.core_or_unavailable()?
            .export_session_to(session_id, destination)
            .await
    }

    /// The file name the save dialog suggests for a session's export (the title, made safe).
    pub async fn export_session_name(&self, session_id: &str) -> String {
        match self.core_or_unavailable() {
            Ok(core) => core.export_session_name(session_id).await,
            Err(_) => "Vibe session.md".to_owned(),
        }
    }
}
