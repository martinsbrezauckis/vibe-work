//! Bounded preview screenshots for the core and browser capture pipeline.
//! Depends on the service visibility checks and native engine; preserves mount and activity locking.
use super::{PreviewError, PreviewService, bounded};
use crate::engine::EngineError;
use crate::shot::Shot;

impl PreviewService {
    /// A shaped screenshot of the session's visible preview.
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], engine failures and timeouts, or [`PreviewError::Image`].
    pub async fn screenshot(&self, session_id: &str) -> Result<Shot, PreviewError> {
        self.screenshot_of_mount(session_id)
            .await
            .map(|(shot, _)| shot)
    }

    /// The engine's own PNG of what the session's preview shows, at full size for stitching and comparing.
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], engine failures and timeouts.
    pub async fn screenshot_png(&self, session_id: &str) -> Result<Vec<u8>, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        if self.shots_blocked(session_id) {
            return Err(PreviewError::Image(crate::browse::SHOTS_BLOCKED.to_owned()));
        }
        self.require_visible_screenshot(session_id)?;
        let id = self.ensure_mounted(session_id).await?;
        Ok(bounded(
            self.config.shot_timeout,
            "screenshot",
            self.engine.screenshot(&id),
        )
        .await?)
    }

    /// A full-page capture's viewport slice, at the page's real resolution where supported.
    ///
    /// # Errors
    /// Nothing open, a hidden/protected page, engine failures and timeouts.
    pub async fn screenshot_slice_png(&self, session_id: &str) -> Result<Vec<u8>, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        if self.shots_blocked(session_id) {
            return Err(PreviewError::Image(crate::browse::SHOTS_BLOCKED.to_owned()));
        }
        self.require_visible_screenshot(session_id)?;
        let id = self.ensure_mounted(session_id).await?;
        Ok(bounded(
            self.config.shot_timeout,
            "viewport slice",
            self.engine.screenshot_slice(&id),
        )
        .await?)
    }

    /// The engine's PNG of the whole document of CSS size `size` in one render, `None` when the engine has no such
    /// capture (macOS; the caller stitches).
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`], engine failures and timeouts.
    pub async fn screenshot_full_png(
        &self,
        session_id: &str,
        size: (u32, u32),
    ) -> Result<Option<Vec<u8>>, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        if self.shots_blocked(session_id) {
            return Err(PreviewError::Image(crate::browse::SHOTS_BLOCKED.to_owned()));
        }
        self.require_visible_screenshot(session_id)?;
        let id = self.ensure_mounted(session_id).await?;
        match bounded(
            self.config.shot_timeout * 3,
            "full-page screenshot",
            self.engine.screenshot_full(&id, size),
        )
        .await
        {
            Ok(png) => Ok(Some(png)),
            Err(EngineError::Unsupported(_)) => Ok(None),
            Err(other) => Err(other.into()),
        }
    }
}
