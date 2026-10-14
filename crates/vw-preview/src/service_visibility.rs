//! Screenshot admission using the window report, native state, page selection and server status.
//! Depends on: preview state and visibility picker. Called by `PreviewService` screenshot paths.

use super::{BrowserMountState, PreviewError, PreviewService};
use crate::visibility;

impl PreviewService {
    /// Only a preview shown in the native window has trustworthy screenshot pixels.
    pub(super) fn require_visible_screenshot(&self, session_id: &str) -> Result<(), PreviewError> {
        let state = self.inner.lock();
        let Some(entry) = state.entries.get(session_id) else {
            return Err(PreviewError::NothingOpen);
        };
        let Some(url) = entry.url.as_ref() else {
            return Err(PreviewError::NothingOpen);
        };
        let owner = state.owner_of(session_id);
        let label = if owner == session_id {
            "App preview".to_owned()
        } else {
            let title = entry.title.as_deref().unwrap_or("untitled");
            format!("web tab '{}'", title.chars().take(80).collect::<String>())
        };
        let report = self
            .visibility
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *self
            .native_window_visible
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            == Some(false)
        {
            return Err(PreviewError::ScreenshotHidden(format!(
                "The Vibe Work window is minimized or hidden, so {label} cannot be captured. Ask the user to show the window, or check without a screenshot (browser_read text)."
            )));
        }
        if let Some(reason) = visibility::screenshot_reason(
            report.as_ref(),
            &owner,
            session_id,
            &label,
            entry.mount.is_none()
                && entry.server_status.is_none()
                && entry.browser_mount != BrowserMountState::Waiting,
            state.is_shown(session_id) || entry.server_status.is_some(),
        ) {
            return Err(PreviewError::ScreenshotHidden(reason));
        }
        if entry.server_status.is_some() {
            return Err(PreviewError::ScreenshotHidden(format!(
                "{label} for {url} remains stopped. Start its server if needed, then Reload."
            )));
        }
        Ok(())
    }
}
