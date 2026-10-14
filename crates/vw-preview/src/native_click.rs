//! Host-authorized native pointer input on a mounted preview. Depends on the engine and mount admission;
//! browser actions call this only after their live permission/confirmation checks. Never remounts or shows a view.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, atomic::AtomicBool};

use crate::{EngineError, MountId, PreviewError, PreviewService};

/// A point in the top document's CSS viewport, after all ancestor-frame offsets.
/// The native engine maps viewport fractions to its current widget pixels/points, including zoom and DPI.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativeClickPoint {
    /// CSS x from the viewport's left edge.
    pub x: f64,
    /// CSS y from the viewport's top edge.
    pub y: f64,
    /// CSS viewport width used by the live page measurement.
    pub viewport_width: f64,
    /// CSS viewport height used by the live page measurement.
    pub viewport_height: f64,
}

/// Host-only validation and dispatch evidence; browser tool arguments cannot supply these fields.
#[derive(Clone, Default)]
pub struct NativeClickGuard {
    /// Boolean page expression checked on the native UI thread immediately before dispatch.
    pub validation: Option<String>,
    /// Set when a native press is sent; survives cancellation of the awaiting action.
    pub sent: Arc<AtomicBool>,
    /// Host permission recheck on the native UI thread immediately before input; never supplied by the page.
    pub allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl PreviewService {
    /// Injects into exactly the admitted mount; the engine must refuse a hidden view or a foreign target window.
    /// The caller supplies host authorization, never an agent-supplied coordinate.
    ///
    /// # Errors
    /// Closed/replaced mount, unsupported engine, hidden/foreign window, or native dispatch failure.
    pub async fn native_click_on_mount(
        &self,
        id: &MountId,
        point: NativeClickPoint,
        validation: String,
        sent: Arc<AtomicBool>,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<(), PreviewError> {
        let _activity = self.session_lock(&id.session_id).await;
        if self.page_mount(&id.session_id).as_ref() != Some(id) {
            return Err(PreviewError::Engine(EngineError::NotMounted));
        }
        self.touch_pool(&id.session_id);
        self.bounded(
            "native click",
            self.engine.native_click(
                id,
                point,
                NativeClickGuard {
                    validation: Some(validation),
                    sent,
                    allowed,
                },
            ),
        )
        .await?;
        Ok(())
    }
}
