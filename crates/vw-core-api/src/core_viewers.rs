//! Opens native viewers and resets their placements when the main document is replaced.
//! Depends on `CoreConfig` and viewer hubs; called by core startup and the host's page-load hook.

use crate::core::CoreConfig;
use std::sync::Arc;

#[cfg(test)]
#[path = "core_viewers_tests.rs"]
mod tests;

/// Clears the placements owned by the previous main document, retaining its live pages.
pub(crate) async fn reset(
    preview: Option<&vw_preview::PreviewService>,
    pages: Option<&vw_pages::PagesService>,
    files: Option<&vw_pages::PagesService>,
) {
    if let Some(service) = preview
        && let Err(error) = service.reset_ui_placement().await
    {
        tracing::warn!(%error, "could not reset Browser placement after UI load");
    }
    for service in [pages, files].into_iter().flatten() {
        if let Err(error) = service.layout(None).await {
            tracing::warn!(%error, "could not reset viewer placement after UI load");
        }
    }
}

impl crate::Core {
    /// Hides native children left by a previous main document without closing their pages.
    pub async fn reset_native_views(&self) {
        reset(
            self.shared.preview.as_ref().map(|hub| &hub.service),
            self.shared.pages.as_ref().map(|hub| &hub.service),
            self.shared.files.viewer.as_ref().map(|hub| &hub.service),
        )
        .await;
    }
}

/// The native-view features the app configured: previews, the Project tab's viewer and the File tab's
/// viewer.
pub(crate) fn open(
    config: &CoreConfig,
) -> (
    Option<Arc<crate::previews::PreviewHub>>,
    Option<Arc<crate::pages::PagesHub>>,
    crate::files::FilesState,
) {
    (
        crate::previews::open(config.preview.clone()),
        crate::pages::open(config.pages.clone()),
        crate::files::FilesState::new(crate::pages::open(config.files_viewer.clone())),
    )
}
