//! Chat-owned native browser windows, each containing one independently owned web page.
//! Core checks agent authority; the service keeps windows out of panel layout and background suspension.
//! Depends on: preview mount lifecycle and native window engines.
//! Called by: core browser-window commands.
use crate::{EngineError, PreviewError, PreviewService};
use serde::Serialize;
use std::sync::Arc;

/// Native window operations; only core-created web pages may acquire a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageWindowAction {
    /// Create an unfocused window before mounting its page.
    Open,
    /// Bring this window to the foreground.
    Focus,
    /// Bring the main window containing a checked panel page to the foreground.
    FocusPanel,
    /// Close the empty window after its page has been unmounted.
    Close,
    /// Set logical content size, then read the actual native dimensions.
    Resize {
        /// Requested logical width.
        width: u32,
        /// Requested logical height.
        height: u32,
    },
    /// Read native visibility, focus and logical content size.
    Inspect,
}

/// Host facts about a browser window, never page-provided geometry or text.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageWindowInfo {
    /// Opaque id scoped to the page's chat.
    pub window_id: String,
    /// Native focus was confirmed.
    pub focused: bool,
    /// Window is visible and not minimized.
    pub visible: bool,
    /// Logical content width.
    pub width: f64,
    /// Logical content height.
    pub height: f64,
}

pub(crate) type CloseListener = Arc<dyn Fn(String) + Send + Sync>;

pub(crate) fn focusable_id(state: &crate::state::State, page: &str) -> Option<String> {
    state.page_owners.get(page)?;
    Some(
        state
            .windows
            .get(page)
            .cloned()
            .unwrap_or_else(|| format!("panel-{}", page.strip_prefix("web-page-").unwrap_or(page))),
    )
}

impl PreviewService {
    /// Installs the person's native window-close route; core owns asynchronous tab cleanup.
    pub fn on_window_close(&self, handler: impl Fn(String) + Send + Sync + 'static) {
        *self
            .inner
            .window_close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(handler));
    }

    /// The window id for a page, if the page has its own window.
    #[must_use]
    pub fn page_window_id(&self, page: &str) -> Option<String> {
        self.inner.lock().windows.get(page).cloned()
    }

    /// A chat-scoped focus handle for a web page, including pages in the main Browser panel.
    /// Panel handles select their own tab and never authorize closing the main window.
    #[must_use]
    pub fn focusable_window_id(&self, page: &str) -> Option<String> {
        focusable_id(&self.inner.lock(), page)
    }

    /// Resolves only a window belonging to the requested chat.
    #[must_use]
    pub fn window_page(&self, owner: &str, window: &str) -> Option<String> {
        let state = self.inner.lock();
        state
            .page_owners
            .iter()
            .find(|(page, found)| {
                *found == owner && focusable_id(&state, page).as_deref() == Some(window)
            })
            .map(|(page, _)| page.clone())
    }

    /// Creates a window for a new web page, before its first navigation.
    ///
    /// # Errors
    /// Foreign/pinned/already-mounted pages, or native window creation failure.
    pub async fn open_page_window(
        &self,
        owner: &str,
        page: &str,
    ) -> Result<PageWindowInfo, PreviewError> {
        let _activity = self.session_lock(page).await;
        {
            let state = self.inner.lock();
            if state
                .page_owners
                .get(page)
                .is_none_or(|found| found != owner)
                || state
                    .entries
                    .get(page)
                    .is_some_and(|entry| entry.mount.is_some())
                || state.windows.contains_key(page)
            {
                return Err(PreviewError::InvalidTab(
                    "Open a new web page for the chat window.".into(),
                ));
            }
        }
        let info = self
            .bounded(
                "open browser window",
                self.engine.page_window(page, PageWindowAction::Open),
            )
            .await?
            .ok_or_else(|| {
                EngineError::Platform("The native browser window was not created.".into())
            })?;
        self.inner
            .lock()
            .windows
            .insert(page.into(), info.window_id.clone());
        self.inner.changed(owner);
        Ok(info)
    }

    /// Focuses an existing chat window, selecting its tab when it belongs to the visible Browser panel.
    ///
    /// # Errors
    /// Unknown/foreign window or native focus failure.
    pub async fn focus_page_window(
        &self,
        owner: &str,
        window: &str,
    ) -> Result<PageWindowInfo, PreviewError> {
        let page = self.window_page(owner, window).ok_or_else(|| {
            PreviewError::InvalidTab("This window does not belong to this chat.".into())
        })?;
        let action = if self.page_window_id(&page).is_some() {
            PageWindowAction::Focus
        } else {
            if !self.front_tab_if_visible(owner, &page).await? {
                return Err(PreviewError::InvalidTab(
                    "This chat's Browser panel is hidden or covered. Ask the user to show this chat's Browser tab, then focus its windowId again.".into(),
                ));
            }
            PageWindowAction::FocusPanel
        };
        let _activity = self.session_lock(&page).await;
        let mut info = self
            .bounded(
                "focus browser window",
                self.engine.page_window(&page, action),
            )
            .await?
            .ok_or_else(|| {
                PreviewError::InvalidTab(
                    "This browser window is closed. Use a current windowId from list_tabs.".into(),
                )
            })?;
        window.clone_into(&mut info.window_id);
        Ok(info)
    }

    /// Reads native facts for this chat's independent windows without selecting or focusing a page.
    ///
    /// # Errors
    /// A native inspection failure or a window that closed while being inspected.
    pub async fn inspect_page_windows(
        &self,
        owner: &str,
    ) -> Result<Vec<PageWindowInfo>, PreviewError> {
        let mut windows = Vec::new();
        for page in self.web_tabs(owner) {
            let _activity = self.session_lock(&page).await;
            if self.owner_session_id(&page).as_deref() != Some(owner) {
                continue;
            }
            if let Some(info) = self.native_window_info(&page).await? {
                windows.push(info);
            }
        }
        Ok(windows)
    }

    pub(crate) async fn native_window_info(
        &self,
        page: &str,
    ) -> Result<Option<PageWindowInfo>, PreviewError> {
        if self.page_window_id(page).is_none() {
            return Ok(None);
        }
        Ok(self
            .bounded(
                "inspect browser window",
                self.engine.page_window(page, PageWindowAction::Inspect),
            )
            .await?)
    }

    /// Caller holds the page operation lock and has already unmounted the webview.
    pub(crate) async fn close_page_window(&self, page: &str) -> Result<(), PreviewError> {
        if self.page_window_id(page).is_some() {
            self.bounded(
                "close browser window",
                self.engine.page_window(page, PageWindowAction::Close),
            )
            .await?;
            self.inner.lock().windows.remove(page);
        }
        Ok(())
    }
}
