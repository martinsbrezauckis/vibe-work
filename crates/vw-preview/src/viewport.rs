//! Explicit responsive viewport placement, kept with the page across layout and navigation.
//! Called by browser tools; the panel's own width control clears this override.
use crate::{PreviewError, PreviewService, layout};

impl PreviewService {
    /// Changes the mounted page's native light/dark preference and retains it across remounts.
    ///
    /// # Errors
    /// Closed page, unsupported engine or native failure.
    pub async fn set_color_scheme(
        &self,
        session: &str,
        dark: Option<bool>,
    ) -> Result<(), PreviewError> {
        let _activity = self.session_lock(session).await;
        let id = self.page_mount(session).ok_or(PreviewError::NothingOpen)?;
        self.bounded("change page color", self.engine.color_scheme(&id, dark))
            .await?;
        self.inner.lock().entry(session).color_scheme = dark;
        Ok(())
    }

    pub(crate) fn mount_with_color(&self, spec: crate::MountSpec) -> crate::EngineFuture<'_, ()> {
        Box::pin(async move {
            let id = spec.id.clone();
            let dark = self.inner.lock().entry(&id.session_id).color_scheme;
            self.engine.mount(spec).await?;
            if dark.is_some() {
                self.engine.color_scheme(&id, dark).await?;
            }
            Ok(())
        })
    }

    /// Sets the page's CSS viewport, or restores its selected device width.
    ///
    /// # Errors
    /// Invalid size, closed page, independent window, or native placement failure.
    pub async fn set_viewport(
        &self,
        session: &str,
        viewport: Option<(u32, u32)>,
    ) -> Result<(), PreviewError> {
        if viewport.is_some_and(|(w, h)| !(240..=4096).contains(&w) || !(240..=4096).contains(&h)) {
            return Err(PreviewError::Layout(
                "Viewport width and height must be 240–4096 CSS pixels.".into(),
            ));
        }
        let _activity = self.session_lock(session).await;
        let (mount, placement, shown) = {
            let state = self.inner.lock();
            if state.windows.contains_key(session) {
                return Err(PreviewError::Layout("This page fills an independent window; use window.resize or select a Browser panel tab for responsive sizes.".into()));
            }
            let entry = state
                .entries
                .get(session)
                .ok_or(PreviewError::NothingOpen)?;
            let mount = entry
                .ready_mount()
                .ok_or(PreviewError::NothingOpen)?
                .clone();
            let shown = state.shown.as_ref().filter(|s| s.session_id == session);
            let area = shown
                .map(|s| s.area)
                .or(state.last_area)
                .unwrap_or(layout::PARKED_AREA);
            let placement = layout::place_viewport(area, entry.width, viewport)
                .map_err(PreviewError::Layout)?;
            (mount, placement, shown.is_some())
        };
        let id = self.mount_id(session, &mount);
        if shown {
            self.bounded("resize viewport", self.engine.place(&id, Some(placement)))
                .await?;
        } else {
            self.bounded("resize hidden viewport", self.engine.park(&id, placement))
                .await?;
        }
        self.inner.lock().entry(session).viewport = viewport;
        self.inner.changed(session);
        Ok(())
    }
}
