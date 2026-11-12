//! Bounded resizing of an existing chat-owned browser window; the main application window is excluded.
//! Core supplies ownership and Stop admission; the native engine returns measured dimensions.
use crate::{PageWindowAction, PageWindowInfo, PreviewError, PreviewService};
impl PreviewService {
    /// Resizes only this chat's independent browser window.
    ///
    /// # Errors
    /// Foreign/main/closed window, invalid size, or native resize failure.
    pub async fn resize_page_window(
        &self,
        owner: &str,
        window: &str,
        width: u32,
        height: u32,
    ) -> Result<PageWindowInfo, PreviewError> {
        let page = self.window_page(owner, window).ok_or_else(|| {
            PreviewError::InvalidTab(
                "This window does not belong to this chat; use its windowId from list_tabs.".into(),
            )
        })?;
        let _activity = self.session_lock(&page).await;
        if self.page_window_id(&page).as_deref() != Some(window) {
            return Err(PreviewError::InvalidTab("This page is in the main window; use tabs.emulate on its Browser panel tab instead.".into()));
        }
        if !(400..=4096).contains(&width) || !(300..=4096).contains(&height) {
            return Err(PreviewError::Layout("Window width must be 400–4096 and height 300–4096 logical pixels; choose a size within those bounds.".into()));
        }
        self.bounded(
            "resize browser window",
            self.engine
                .page_window(&page, PageWindowAction::Resize { width, height }),
        )
        .await?
        .ok_or_else(|| {
            PreviewError::InvalidTab(
                "The browser window closed; use a current windowId from list_tabs.".into(),
            )
        })
    }
}
