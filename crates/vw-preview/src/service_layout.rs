//! Applies a shown preview layout after the service records the desired session.
//! Depends on the preview engine, placement model and mount state; called by `PreviewService::layout`.

use super::{PreviewError, PreviewService};
use crate::engine::MountId;
use crate::layout::{self, Rect};
use crate::state::{BrowserMountState, Shown};

/// Native-only placement preserves panel ownership; a panel report replaces or clears it.
pub(super) enum PanelSelection {
    Unchanged,
    Set(Option<String>),
}

impl PreviewService {
    /// Hides every live child and forgets placements and chrome handoffs from the documents before main-document
    /// load `generation`; their window layouts that arrive later place nothing.
    /// Pages, tab selection, addresses and agent ownership remain alive.
    ///
    /// # Errors
    /// The first engine failure, after attempting to hide every child.
    pub async fn reset_ui_placement(&self, generation: u64) -> Result<(), PreviewError> {
        let _layout = self.layout_op.lock().await;
        self.placement_gate.advance(generation);
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

    /// A window layout of a main document a reset replaced: it places nothing. The caller holds `layout_op`, so
    /// one admitted here finishes before the reset hides it.
    pub(super) fn stale_window_layout(&self) -> bool {
        let stale = !self.placement_gate.admits();
        if stale {
            tracing::debug!("layout ignored: its main document was replaced");
        }
        stale
    }

    /// The CSS viewport width this service lays `session_id`'s mounted page out at: its placement's width divided by
    /// its zoom (the shown placement, else the parked one), `None` when nothing is mounted. Our own number, never the
    /// page's (`innerWidth` is page data): the full-page stitch takes its scale from it.
    #[must_use]
    pub fn css_viewport_width(&self, session_id: &str) -> Option<f64> {
        let state = self.inner.lock();
        let entry = state.entries.get(session_id)?;
        entry.ready_mount()?;
        if state.windows.contains_key(session_id) {
            return None;
        }
        let placement = match &state.shown {
            Some(shown) if shown.session_id == session_id => {
                layout::place_viewport(shown.area, entry.width, entry.viewport).ok()?
            }
            _ => layout::parked_viewport(state.last_area, entry.width, entry.viewport),
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
        if let Some(window) = self.native_window_info(session_id).await? {
            return Ok(Some((window.width, window.height)));
        }
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
                    layout::place_viewport(shown.area, entry.width, entry.viewport).ok()
                }
                _ => Some(layout::parked_viewport(
                    state.last_area,
                    entry.width,
                    entry.viewport,
                )),
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
                let placement =
                    layout::place_viewport(shown.area, entry.width, entry.viewport).ok()?;
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

impl PreviewService {
    /// What the window shows: `Some((session, area))` places that session's preview there (mounting it again
    /// when suspended) and hides every other one; `None` hides all.
    ///
    /// # Errors
    /// [`PreviewError::Layout`] for an invalid area; engine failures of the shown preview.
    pub async fn layout(&self, shown: Option<(String, Rect)>) -> Result<(), PreviewError> {
        self.layout_selected(shown, PanelSelection::Unchanged).await
    }

    /// Applies a passive UI layout report only while its page is still the selected tab.
    /// A queued report from a tab switched away from cannot restore its native view.
    ///
    /// # Errors
    /// A rectangle without a page, invalid placement, or native engine failure.
    pub async fn report_tab_layout(
        &self,
        page: Option<String>,
        rect: Option<Rect>,
    ) -> Result<(), PreviewError> {
        let _layout = self.layout_op.lock().await;
        if self.stale_window_layout() {
            return Ok(());
        }
        tracing::debug!(
            page = page.as_deref().unwrap_or("-"),
            ?rect,
            "preview layout reported"
        );
        let Some(page_key) = page else {
            if rect.is_some() {
                return Err(PreviewError::InvalidTab(
                    "a Browser rectangle needs its page".to_owned(),
                ));
            }
            return self
                .layout_selected_locked(None, PanelSelection::Set(None))
                .await;
        };
        let current = {
            let state = self.inner.lock();
            let owner = state.owner_of(&page_key);
            state
                .tabs
                .get(&owner)
                .and_then(|tabs| tabs.selected.as_deref())
                .unwrap_or(&owner)
                == page_key.as_str()
        };
        if !current {
            tracing::debug!(page = %page_key, "layout ignored: the page is not its session's selected tab");
            return Ok(());
        }
        // A Browser panel reopening can reclaim an engine released while the page was away.
        // Native-only layout changes intentionally keep Released parked.
        if rect.is_some()
            && let Some(entry) = self.inner.lock().entries.get_mut(&page_key)
            && entry.browser_mount == BrowserMountState::Released
            && entry.server_status.is_none()
        {
            entry.browser_mount = BrowserMountState::Ready;
        }
        self.layout_selected_locked(
            rect.map(|area| (page_key.clone(), area)),
            PanelSelection::Set(Some(page_key)),
        )
        .await
    }

    pub(super) async fn layout_selected(
        &self,
        shown: Option<(String, Rect)>,
        selected: PanelSelection,
    ) -> Result<(), PreviewError> {
        let _layout = self.layout_op.lock().await;
        if self.stale_window_layout() {
            return Ok(());
        }
        self.layout_selected_locked(shown, selected).await
    }

    /// Caller holds `layout_op`; places a selected page at the still-current Browser area.
    pub(crate) async fn layout_page_locked(
        &self,
        page_key: String,
        area: Rect,
    ) -> Result<(), PreviewError> {
        self.layout_selected_locked(
            Some((page_key.clone(), area)),
            PanelSelection::Set(Some(page_key)),
        )
        .await
    }

    /// Caller holds `layout_op`, so visibility checks and native placement share one ordering.
    async fn layout_selected_locked(
        &self,
        shown: Option<(String, Rect)>,
        selected: PanelSelection,
    ) -> Result<(), PreviewError> {
        if let Some((_, area)) = &shown {
            area.checked().map_err(PreviewError::Layout)?;
        }
        if let PanelSelection::Set(session) = selected {
            self.select_browser_panel(session).await;
        }
        // Hiding (including a modal or session switch) parks the view, never its document.
        let previously_shown = self
            .inner
            .lock()
            .shown
            .as_ref()
            .map(|s| s.session_id.clone());
        tracing::debug!(
            shown = shown.as_ref().map_or("-", |(page, _)| page.as_str()),
            previously = previously_shown.as_deref().unwrap_or("-"),
            "preview layout applied"
        );
        if let Some(session) = previously_shown
            && shown.as_ref().is_none_or(|(next, _)| next != &session)
        {
            self.capture_tab_snapshot(&session).await;
            self.touch_pool(&session);
        }
        let (others, target) = {
            let mut state = self.inner.lock();
            state.shown = shown
                .clone()
                .map(|(session_id, area)| Shown { session_id, area });
            if let Some((_, area)) = &shown {
                state.last_area = Some(*area);
            }
            let now = state.tick();
            if shown.is_none() {
                for entry in state.entries.values_mut() {
                    if entry.browser_mount == BrowserMountState::Released {
                        entry.browser_mount = BrowserMountState::Ready;
                    }
                }
            }
            let target = shown.as_ref().and_then(|(session, _)| {
                state.entries.get_mut(session)?.last_used = now;
                Some(session.clone())
            });
            let others: Vec<MountId> = state
                .entries
                .values()
                .filter(|e| shown.as_ref().is_none_or(|(s, _)| *s != e.session_id))
                .filter_map(|e| {
                    e.ready_mount().map(|m| MountId {
                        session_id: e.session_id.clone(),
                        label: m.label.clone(),
                    })
                })
                .collect();
            (others, target)
        };
        for id in others {
            let _activity = self.session_lock(&id.session_id).await;
            let current = self
                .inner
                .lock()
                .entries
                .get(&id.session_id)
                .and_then(|entry| entry.mount.as_ref())
                .is_some_and(|mount| mount.label == id.label);
            if current {
                let _ = self.bounded("hide", self.engine.place(&id, None)).await;
            }
        }
        if let Some(session) = target {
            self.apply_layout_target(&session).await?;
        }
        Ok(())
    }
}
