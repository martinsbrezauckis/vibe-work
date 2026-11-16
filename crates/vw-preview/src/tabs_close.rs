//! Closing a session's web page: the closing marker, the native close, and restoring the page or
//! the session's next page when the close fails or the page was the agent's current one.

use crate::service::{PreviewError, PreviewService};

use super::{PageTabOwner, review_fingerprint};

struct ClosingPage<'a> {
    service: &'a PreviewService,
    key: &'a str,
}

impl Drop for ClosingPage<'_> {
    fn drop(&mut self) {
        self.service.inner.lock().closing_pages.remove(self.key);
    }
}

impl PreviewService {
    async fn close_web_tab_expected(
        &self,
        owner: &str,
        page_key: &str,
        expected: Option<&str>,
        admit: Option<&(dyn Fn() -> bool + Send + Sync)>,
    ) -> Result<(), PreviewError> {
        let _activity = self.session_lock(page_key).await;
        {
            let mut state = self.inner.lock();
            if state
                .page_owners
                .get(page_key)
                .is_none_or(|found| found != owner)
            {
                return Err(PreviewError::InvalidTab(
                    "web page does not belong to this session".to_owned(),
                ));
            }
            if let Some(expected) = expected
                && review_fingerprint(&state, owner, page_key)? != expected
            {
                return Err(PreviewError::InvalidTab(
                    "browser tab changed while closing".to_owned(),
                ));
            }
            if admit.is_some_and(|check| !check()) {
                return Err(PreviewError::InvalidTab(
                    "browser tab close was canceled or its permission changed".to_owned(),
                ));
            }
            if !state.closing_pages.insert(page_key.to_owned()) {
                return Err(PreviewError::InvalidTab(
                    "web page is already closing".to_owned(),
                ));
            }
        }
        let _closing = ClosingPage {
            service: self,
            key: page_key,
        };
        self.unmount(page_key).await?;
        self.close_page_window(page_key).await?;
        {
            let mut state = self.inner.lock();
            if let Some(entry) = state.entries.remove(page_key) {
                for record in entry.downloads {
                    state.entry(owner).push_download(record);
                }
            }
            state.page_owners.remove(page_key);
            let popup_return = crate::popups::closed(&mut state, owner, page_key);
            if state.is_shown(page_key) {
                state.shown = None;
            }
            if let Some(tabs) = state.tabs.get_mut(owner) {
                let closed_at = tabs.web.iter().position(|key| key == page_key).unwrap_or(0);
                tabs.web.retain(|key| key != page_key);
                tabs.owners.remove(page_key);
                tabs.snapshots.remove(page_key);
                if tabs.selected.as_deref() == Some(page_key) {
                    tabs.selected = Some(
                        tabs.web
                            .get(closed_at)
                            .or_else(|| tabs.web.last())
                            .cloned()
                            .unwrap_or_else(|| owner.to_owned()),
                    );
                }
                if tabs.agent.as_deref() == Some(page_key) {
                    tabs.agent = if let Some(opener) = popup_return {
                        match opener {
                            crate::popups::Return::Available(page) => Some(page),
                            crate::popups::Return::Unavailable => None,
                        }
                    } else {
                        tabs.web[closed_at.min(tabs.web.len())..]
                            .iter()
                            .chain(tabs.web[..closed_at.min(tabs.web.len())].iter().rev())
                            .find(|key| {
                                tabs.owners
                                    .get(*key)
                                    .is_some_and(|kind| *kind != PageTabOwner::User)
                            })
                            .cloned()
                    };
                }
            }
        }
        self.refresh_rules().await;
        self.inner.changed(owner);
        Ok(())
    }

    /// Closes a selected page and shows its adjacent tab only while this panel remains visible.
    /// The close, visibility check and replacement share one layout ordering with modal hides.
    ///
    /// # Errors
    /// The page cannot close or the replacement cannot be placed.
    pub async fn close_web_tab_and_restore(
        &self,
        owner: &str,
        page_key: &str,
    ) -> Result<(), PreviewError> {
        match self
            .close_web_tab_and_restore_expected(owner, page_key, None, None)
            .await?
        {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Closes only the exact owner, mount, document and URL previously reviewed.
    /// The admission check runs after the page operation lock and before native teardown.
    /// A successful close returns any error from showing the replacement page separately.
    ///
    /// # Errors
    /// The reviewed page or permission changed, the page is foreign, or its native view cannot close.
    pub async fn close_web_tab_and_restore_checked(
        &self,
        owner: &str,
        page_key: &str,
        fingerprint: &str,
        admit: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<Option<PreviewError>, PreviewError> {
        self.close_web_tab_and_restore_expected(owner, page_key, Some(fingerprint), Some(admit))
            .await
    }

    async fn close_web_tab_and_restore_expected(
        &self,
        owner: &str,
        page_key: &str,
        expected: Option<&str>,
        admit: Option<&(dyn Fn() -> bool + Send + Sync)>,
    ) -> Result<Option<PreviewError>, PreviewError> {
        let _layout = self.layout_op.lock().await;
        let area = {
            let state = self.inner.lock();
            state
                .shown
                .as_ref()
                .filter(|shown| shown.session_id == page_key)
                .map(|shown| shown.area)
        };
        self.close_web_tab_expected(owner, page_key, expected, admit)
            .await?;
        let restore_error = if let Some(area) = area {
            let selected = self
                .tabs(owner)
                .into_iter()
                .find(|tab| tab.active)
                .map_or_else(|| owner.to_owned(), |tab| tab.page_key);
            self.layout_page_locked(selected, area).await.err()
        } else {
            None
        };
        Ok(restore_error)
    }
}
