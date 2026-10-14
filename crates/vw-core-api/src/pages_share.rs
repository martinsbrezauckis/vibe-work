//! Share a page as one file in the core: `exportPage` builds the self-contained HTML (vw-pages `export`)
//! with the page's answers frozen as text, and writes it where the user chose: the window through the native save
//! dialog (the app's `export_page` command, [`Core::export_page_to`]), or the Debug API acting as
//! the user by naming `destination`, like its `pickFolder`. Without a destination the text comes back.
//!
//! Depends on: vw-pages (`export_page`, answers), `pages` (helpers, the session's project). Called by: `pages`
//! (`exportPage`), vw-host (the app's save-dialog command).
//! Invariants: the window cannot name a destination (a path from the webview is never written to); a destination
//! must be absolute, its folder must exist, and the file is written whole (a temporary file renamed over it).

use std::path::Path;

use vw_pages::{Exported, PagesService};

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::pages::{blocking, core_error, path_of};
use crate::pages_view::PagesResult;

impl Core {
    /// `exportPage` (see the module header).
    pub(crate) async fn export_page(
        &self,
        service: &PagesService,
        origin: Origin,
        session_id: &str,
        path: &str,
        destination: Option<String>,
    ) -> Result<PagesResult, CoreError> {
        if destination.is_some() && origin != Origin::DebugApi {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "the window saves an exported page through the native save dialog",
            ));
        }
        let exported = self.build_export(service, session_id, path).await?;
        match destination {
            None => Ok(result(exported, None, true)),
            Some(destination) => {
                let target = write_export(&exported, Path::new(&destination)).await?;
                Ok(result(exported, Some(target), false))
            }
        }
    }

    /// Exports `path` of `session_id`'s handbook to `destination`, a file the user chose in the native save dialog
    /// (the app's command; the webview never supplies the path).
    ///
    /// # Errors
    /// `notFound` for an unknown session or page, `invalidParams` for a bad destination, `internal` when the file
    /// cannot be written.
    pub async fn export_page_to(
        &self,
        session_id: &str,
        path: &str,
        destination: &Path,
    ) -> Result<PagesResult, CoreError> {
        let service = self
            .shared
            .pages
            .as_ref()
            .map(|hub| hub.service.clone())
            .ok_or_else(|| {
                CoreError::not_in_this_build("Project Pages are not available in this build")
            })?;
        let exported = self.build_export(&service, session_id, path).await?;
        let target = write_export(&exported, destination).await?;
        Ok(result(exported, Some(target), false))
    }

    /// The suggested file name of an export of `path` (for the save dialog).
    #[must_use]
    pub fn export_name(path: &str) -> String {
        path.rsplit('/').next().unwrap_or("page.html").to_owned()
    }

    async fn build_export(
        &self,
        service: &PagesService,
        session_id: &str,
        path: &str,
    ) -> Result<Exported, CoreError> {
        let project = self.project_of(session_id)?;
        let page = path_of(path)?;
        let owned = service.clone();
        blocking(move || {
            let facts = owned.page(&project, &page)?;
            let answers = (!facts.inputs.is_empty())
                .then(|| owned.answers_of(&project, &page).ok())
                .flatten();
            vw_pages::export_page(&owned.pages_dir(&project), &page, answers.as_ref())
                .map_err(vw_pages::PagesError::Other)
        })
        .await?
        .map_err(|e| core_error(&e))
    }
}

fn result(exported: Exported, saved_to: Option<String>, with_html: bool) -> PagesResult {
    PagesResult::Exported {
        name: exported.name,
        bytes: exported.html.len() as u64,
        inlined: exported.inlined,
        left_out: exported.left_out,
        saved_to,
        html: with_html.then_some(exported.html),
    }
}

/// Writes the export to `destination` (see `save_file::write_whole`).
async fn write_export(exported: &Exported, destination: &Path) -> Result<String, CoreError> {
    crate::save_file::write_whole(exported.html.clone().into_bytes(), destination).await
}
