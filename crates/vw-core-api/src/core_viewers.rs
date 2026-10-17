//! Opens the configured native viewers for core startup. Depends on `CoreConfig` and viewer hubs.
//! Called only by `Core::start`; absent configuration keeps each viewer disabled.

use crate::core::CoreConfig;
use std::sync::Arc;

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
