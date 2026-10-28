//! Browser chrome visibility and capture guard for native child webviews.
//! Depends on Tauri visibility lock and platform capture; called by the preview engine trait.

use super::TauriEngine;
#[cfg(windows)]
use super::UNSHOWN_SIZE;
use crate::platform;
use std::sync::PoisonError;
use std::time::Duration;
use vw_preview::{EngineError, MountId};

impl TauriEngine {
    pub(super) async fn pdf(&self, id: MountId) -> Result<Vec<u8>, EngineError> {
        let webview = self.webview(&id)?;
        let _visibility = self.visibility.lock().await;
        crate::pdf::print(&webview).await
    }

    /// A screenshot of a mount; overlay-hidden pages refuse capture on every platform.
    pub(super) async fn shot(
        &self,
        id: MountId,
        full: Option<(u32, u32)>,
        limit: Duration,
        sharp: bool,
    ) -> Result<Vec<u8>, EngineError> {
        let webview = self.webview(&id)?;
        let visibility = self.visibility.clone().lock_owned().await;
        let shown = self
            .shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id.label)
            .copied()
            .unwrap_or_default();
        if shown.overlay_hidden {
            return Err(EngineError::Unsupported(
                "Browser page capture is off while chrome covers it".to_owned(),
            ));
        }
        #[cfg(target_os = "linux")]
        if sharp {
            return crate::linux_capture::screenshot(&webview, limit, visibility).await;
        }
        // macOS: the same viewport rendered at one point per CSS pixel; nothing about the webview changes.
        #[cfg(target_os = "macos")]
        if sharp {
            let _visibility = visibility;
            return platform::screenshot_sharp(&webview, limit).await;
        }
        // The scaled Linux capture transfers this lock to its UI task so Stop cannot interrupt restoration.
        let _visibility = visibility;
        let _ = sharp;
        #[cfg(windows)]
        {
            if shown.visible {
                platform::screenshot(&webview, limit, full).await
            } else {
                platform::screenshot_hidden(
                    &webview,
                    shown.size.unwrap_or(UNSHOWN_SIZE),
                    limit,
                    full,
                )
                .await
            }
        }
        #[cfg(not(windows))]
        {
            let _ = shown;
            platform::screenshot(&webview, limit, full).await
        }
    }

    /// Parks only the selected native child; a later normal placement restores current bounds.
    pub(super) async fn set_browser_overlay(
        &self,
        id: MountId,
        hidden: bool,
    ) -> Result<(), EngineError> {
        let webview = self.webview(&id)?;
        let _visibility = self.visibility.lock().await;
        #[cfg(target_os = "linux")]
        let _ = platform::trace_browser_overlay(&webview, hidden, false).await;
        if hidden {
            let placement = self
                .shown
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id.label)
                .and_then(|shown| shown.placement)
                .ok_or_else(|| {
                    EngineError::Unsupported("Browser page has no on-screen placement".to_owned())
                })?;
            // Keep subsequent layout calls from raising this child even if native parking
            // fails after it has changed visibility or bounds.
            {
                let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
                let entry = shown.entry(id.label.clone()).or_default();
                entry.overlay_hidden = true;
                entry.visible = false;
            }
            // The maintained platform park keeps the page's viewport dimensions while removing
            // its native pixels from the chrome slot on Linux, Windows and macOS.
            if let Err(error) = platform::park_browser_overlay(&webview, placement).await {
                // If native positioning failed partway through, an explicit hide is the safest
                // fallback. The overlay flag remains set so later layout calls cannot show it.
                let _ = webview.hide();
                return Err(error);
            }
            if std::env::var_os("VW_NATIVE_LAYOUT_TRACE").is_some() {
                tracing::info!(
                    page = %id.session_id,
                    label = %id.label,
                    native_x = platform::BROWSER_PARKED_OFFSET,
                    native_y = platform::BROWSER_PARKED_OFFSET,
                    width = placement.bounds.width,
                    height = placement.bounds.height,
                    "browser overlay child parked"
                );
            }
        } else {
            let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = shown.entry(id.label.clone()).or_default();
            entry.overlay_hidden = false;
        }
        #[cfg(target_os = "linux")]
        let _ = platform::trace_browser_overlay(&webview, hidden, true).await;
        if !hidden && std::env::var_os("VW_NATIVE_LAYOUT_TRACE").is_some() {
            tracing::info!(page = %id.session_id, label = %id.label, "browser overlay child released for current layout");
        }
        Ok(())
    }
}
