//! Browser chrome handoff: park the selected native child while HTML menus cover its slot.
//! Depends on the preview registry, layout lock and engine visibility seam; called by the core's Browser overlay op.

use super::{PreviewError, PreviewService};
use crate::engine::MountId;
use crate::layout::{self, Rect};

/// The selected native child's rectangle in window logical pixels.
pub struct BrowserOverlayPrepared {
    /// Kept for the typed command's compatibility; parked pages never expose copied pixels.
    pub snapshot: Option<String>,
    /// Native page bounds, which can be narrower than the Browser panel at a device width.
    pub bounds: Rect,
}

impl PreviewService {
    fn overlay_target(&self, owner: &str, page: &str) -> Result<(MountId, Rect), PreviewError> {
        let state = self.inner.lock();
        if state.owner_of(page) != owner {
            return Err(PreviewError::InvalidTab(
                "page belongs to another session".to_owned(),
            ));
        }
        let selected = state
            .tabs
            .get(owner)
            .and_then(|tabs| tabs.selected.as_deref())
            .unwrap_or(owner);
        if selected != page || !state.is_shown(page) {
            return Err(PreviewError::InvalidTab(
                "page is no longer selected and visible".to_owned(),
            ));
        }
        let entry = state.entries.get(page).ok_or(PreviewError::NothingOpen)?;
        let mount = entry.mount.as_ref().ok_or(PreviewError::NothingOpen)?;
        let area = state
            .shown
            .as_ref()
            .ok_or_else(|| PreviewError::ScreenshotHidden("Browser page is not shown".to_owned()))?
            .area;
        let bounds = layout::place(area, entry.width)
            .map_err(PreviewError::Layout)?
            .bounds;
        Ok((self.mount_id(page, mount), bounds))
    }

    /// Pins the selected child identity and bounds before the chrome opens.
    ///
    /// # Errors
    /// Returns an error when the page is no longer selected, mounted or placeable.
    pub async fn prepare_browser_overlay(
        &self,
        owner: &str,
        page: &str,
    ) -> Result<BrowserOverlayPrepared, PreviewError> {
        let _layout = self.layout_op.lock().await;
        let _activity = self.session_lock(page).await;
        let (id, bounds) = self.overlay_target(owner, page)?;
        let loads = self
            .inner
            .lock()
            .entries
            .get(page)
            .map(|entry| *entry.loads.borrow())
            .ok_or(PreviewError::NothingOpen)?;
        self.inner
            .lock()
            .browser_overlay_prepared
            .insert(page.to_owned(), (id.label, loads, bounds));
        Ok(BrowserOverlayPrepared {
            snapshot: None,
            bounds,
        })
    }

    /// Parks the selected native child without changing its document, scroll or registry ownership.
    ///
    /// # Errors
    /// Returns an error if preparation is stale or the native child cannot be hidden.
    pub async fn show_browser_overlay(&self, owner: &str, page: &str) -> Result<(), PreviewError> {
        let _layout = self.layout_op.lock().await;
        let (id, bounds) = self.overlay_target(owner, page)?;
        let (current, prepared) = {
            let state = self.inner.lock();
            (
                state.entries.get(page).map(|entry| *entry.loads.borrow()),
                state.browser_overlay_prepared.get(page).cloned(),
            )
        };
        let Some((label, loads, prepared_bounds)) = prepared else {
            return Err(PreviewError::InvalidTab(
                "page overlay was not prepared".to_owned(),
            ));
        };
        if label != id.label || Some(loads) != current || prepared_bounds != bounds {
            return Err(PreviewError::InvalidTab(
                "page changed after overlay preparation".to_owned(),
            ));
        }
        self.inner
            .lock()
            .browser_overlays
            .insert(page.to_owned(), id.label.clone());
        // A previous close may have released the native child before placement failed.
        // Reassert parking even when this mount already owns the overlay record.
        if let Err(error) = self
            .bounded(
                "park browser under chrome",
                self.engine.browser_overlay(&id, true),
            )
            .await
        {
            // A timed-out or failed hide may already have hidden the native child. Keep its
            // identity so the caller's close can still restore it.
            return Err(error.into());
        }
        Ok(())
    }

    /// Ends a menu handoff; a switched or closed tab is never raised over the new selection.
    ///
    /// # Errors
    /// Returns an error if the native child cannot be restored, placed or reparked.
    pub async fn close_browser_overlay(&self, owner: &str, page: &str) -> Result<(), PreviewError> {
        let _layout = self.layout_op.lock().await;
        if self.inner.lock().owner_of(page) != owner {
            return Ok(());
        }
        let label = self.inner.lock().browser_overlays.get(page).cloned();
        let Some(label) = label else {
            self.inner.lock().browser_overlay_prepared.remove(page);
            return Ok(());
        };
        let id = MountId {
            session_id: page.to_owned(),
            label,
        };
        let current_mount = self
            .inner
            .lock()
            .entries
            .get(page)
            .and_then(|e| e.mount.as_ref())
            .is_some_and(|m| m.label == id.label);
        if !current_mount {
            let mut state = self.inner.lock();
            state.browser_overlays.remove(page);
            state.browser_overlay_prepared.remove(page);
            return Ok(());
        }
        // A native visibility call or placement can fail after changing the child. Repark on
        // each failure before releasing the layout lock, so another layout cannot raise it.
        // Keep the overlay record if both attempts fail so a later close can try again.
        for attempt in 0..2 {
            let restored = self
                .bounded(
                    "end browser overlay",
                    self.engine.browser_overlay(&id, false),
                )
                .await;
            let result = match restored {
                Ok(()) if self.overlay_target(owner, page).is_ok() => {
                    self.apply_layout_target(page).await
                }
                Ok(()) => Ok(()),
                Err(error) => Err(error.into()),
            };
            match result {
                Ok(()) => break,
                Err(error) => {
                    self.bounded(
                        "repark browser after failed overlay restore",
                        self.engine.browser_overlay(&id, true),
                    )
                    .await
                    .map_err(PreviewError::from)?;
                    if attempt == 1 {
                        return Err(error);
                    }
                }
            }
        }
        {
            let mut state = self.inner.lock();
            state.browser_overlays.remove(page);
            state.browser_overlay_prepared.remove(page);
        }
        Ok(())
    }
}
