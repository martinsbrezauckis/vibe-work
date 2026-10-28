//! Applies a shown preview layout after the service records the desired session.
//! Depends on the preview engine, placement model and mount state; called by `PreviewService::layout`.

use super::{PreviewError, PreviewService};
use crate::layout;
use crate::state::BrowserMountState;

/// Native-only placement preserves panel ownership; a panel report replaces or clears it.
pub(super) enum PanelSelection {
    Unchanged,
    Set(Option<String>),
}

impl PreviewService {
    /// Hides every live child and forgets placements and chrome handoffs from the old main document.
    /// Pages, tab selection, addresses and agent ownership remain alive.
    ///
    /// # Errors
    /// The first engine failure, after attempting to hide every child.
    pub async fn reset_ui_placement(&self) -> Result<(), PreviewError> {
        let _layout = self.layout_op.lock().await;
        self.select_browser_panel(None).await;
        let (pages, overlays) = {
            let mut state = self.inner.lock();
            state.shown = None;
            state.browser_overlay_prepared.clear();
            let overlays = std::mem::take(&mut state.browser_overlays);
            let pages: Vec<_> = state.entries.keys().cloned().collect();
            (pages, overlays)
        };
        let mut failure = None;
        for page in pages {
            let _activity = self.session_lock(&page).await;
            // An open already in progress must finish before the hide; its document stays mounted.
            let id = self.inner.lock().entries.get(&page).and_then(|entry| {
                entry
                    .mount
                    .as_ref()
                    .map(|mount| self.mount_id(&page, mount))
            });
            let Some(id) = id else { continue };
            if let Err(error) = self
                .bounded("hide after UI load", self.engine.place(&id, None))
                .await
            {
                failure.get_or_insert(PreviewError::from(error));
            }
            // Releasing the overlay flag does not show the child. Only a fresh layout may do that.
            if overlays.get(&id.session_id) == Some(&id.label)
                && let Err(error) = self
                    .bounded(
                        "release old chrome",
                        self.engine.browser_overlay(&id, false),
                    )
                    .await
            {
                failure.get_or_insert(PreviewError::from(error));
            }
        }
        failure.map_or(Ok(()), Err)
    }

    /// The CSS viewport width this service lays `session_id`'s mounted page out at: its placement's width divided by
    /// its zoom (the shown placement, else the parked one), `None` when nothing is mounted. Our own number, never the
    /// page's (`innerWidth` is page data): the full-page stitch takes its scale from it.
    #[must_use]
    pub fn css_viewport_width(&self, session_id: &str) -> Option<f64> {
        let state = self.inner.lock();
        let entry = state.entries.get(session_id)?;
        entry.ready_mount()?;
        let placement = match &state.shown {
            Some(shown) if shown.session_id == session_id => {
                layout::place(shown.area, entry.width).ok()?
            }
            _ => layout::parked(state.last_area, entry.width),
        };
        (placement.zoom > 0.0 && placement.bounds.width > 0.0)
            .then(|| placement.bounds.width / placement.zoom)
    }

    /// The layout viewport of the session's mounted page in CSS px, from the engine's native frame (no pixels, no
    /// page numbers: a page can redefine `innerWidth`) divided by the zoom it was placed or parked with; the placement
    /// itself when the engine cannot tell. `None` when nothing is mounted. Used by a layout capture's size check.
    ///
    /// # Errors
    /// The engine's failure while reading its frame.
    pub async fn layout_viewport(
        &self,
        session_id: &str,
    ) -> Result<Option<(f64, f64)>, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        let (mount, placement) = {
            let state = self.inner.lock();
            let Some(entry) = state.entries.get(session_id) else {
                return Ok(None);
            };
            let Some(mount) = entry.ready_mount().cloned() else {
                return Ok(None);
            };
            let placement = match &state.shown {
                Some(shown) if shown.session_id == session_id => {
                    layout::place(shown.area, entry.width).ok()
                }
                _ => Some(layout::parked(state.last_area, entry.width)),
            };
            (mount, placement)
        };
        let Some(placement) = placement.filter(|p| p.zoom > 0.0) else {
            return Ok(None);
        };
        let id = self.mount_id(session_id, &mount);
        let frame = self
            .bounded("view size", self.engine.view_size(&id))
            .await?
            .unwrap_or((placement.bounds.width, placement.bounds.height));
        Ok(Some((frame.0 / placement.zoom, frame.1 / placement.zoom)))
    }

    pub(super) async fn apply_layout_target(&self, session: &str) -> Result<(), PreviewError> {
        let session = session.to_owned();
        let _activity = self.session_lock(&session).await;
        self.close_unready(&session).await?;
        let current = {
            let state = self.inner.lock();
            state.entries.get(&session).and_then(|entry| {
                let shown = state.shown.as_ref()?;
                let placement = layout::place(shown.area, entry.width).ok()?;
                Some((
                    entry.mount.clone(),
                    entry.url.clone(),
                    placement,
                    entry.server_status.is_some(),
                ))
            })
        };
        let Some((mount, url, placement, stopped)) = current else {
            return Ok(());
        };
        if stopped {
            if let Some(mount) = mount {
                let id = self.mount_id(&session, &mount);
                self.bounded("hide", self.engine.place(&id, None)).await?;
            }
            return Ok(());
        }
        match (mount, url) {
            (Some(mount), _) => {
                self.touch_pool(&session);
                let id = self.mount_id(&session, &mount);
                self.bounded("place", self.engine.place(&id, Some(placement)))
                    .await?;
            }
            (None, Some(url)) => {
                let held = self
                    .inner
                    .lock()
                    .entries
                    .get(&session)
                    .is_some_and(|entry| entry.browser_mount != BrowserMountState::Ready);
                if held {
                    return Ok(());
                }
                self.mount(&session, url, Some(placement)).await?;
                self.enforce_cap(&session).await;
            }
            (None, None) => {}
        }
        Ok(())
    }
}
