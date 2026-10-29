//! Opens native viewers and resets their placements when the main document is replaced.
//! Depends on `CoreConfig` and viewer hubs; called by core startup and the host's page-load hook.

use crate::core::CoreConfig;
use std::sync::Arc;

#[cfg(test)]
#[path = "core_viewers_tests.rs"]
mod tests;

/// Runs a main-window command issued by main-document load `generation`: once a later load has reset the native
/// placements, its Browser, Project and File viewer placements place nothing.
pub async fn for_window_generation<F: std::future::Future>(
    generation: u64,
    command: F,
) -> F::Output {
    vw_preview::for_window_generation(generation, command).await
}

/// Clears the placements owned by the documents before main-document load `generation`, retaining their live pages.
pub(crate) async fn reset(
    generation: u64,
    preview: Option<&vw_preview::PreviewService>,
    pages: Option<&vw_pages::PagesService>,
    files: Option<&vw_pages::PagesService>,
) {
    if let Some(service) = preview
        && let Err(error) = service.reset_ui_placement(generation).await
    {
        tracing::warn!(%error, "could not reset Browser placement after UI load");
    }
    for service in [pages, files].into_iter().flatten() {
        if let Err(error) = service.reset_ui_placement(generation).await {
            tracing::warn!(%error, "could not reset viewer placement after UI load");
        }
    }
}

impl crate::Core {
    /// Hides native children left by the main documents before load `generation` without closing their pages.
    pub async fn reset_native_views(&self, generation: u64) {
        reset(
            generation,
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
