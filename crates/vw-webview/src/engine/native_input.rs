//! Live native input and page evaluation for the Tauri engine. Depends on the mount registry and visibility lock;
//! `PreviewEngine` calls these bounded operations. The OS adapter validates the active window again on the UI thread.

use super::{DRAIN_WAIT, TauriEngine};
use std::sync::{Mutex, PoisonError};
use vw_preview::{EngineError, MountId, NativeClickGuard, NativeClickPoint};

impl TauriEngine {
    pub(super) async fn native_click_inner(
        &self,
        id: MountId,
        point: NativeClickPoint,
        guard: NativeClickGuard,
    ) -> Result<(), EngineError> {
        let _visibility = self.visibility.lock().await;
        if !self
            .shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id.label)
            .is_some_and(|shown| shown.visible)
        {
            return Err(EngineError::Platform(
                "Not run: show this browser in the app's active window before a native click"
                    .to_owned(),
            ));
        }
        let webview = self.webview(&id)?;
        crate::native_click::native_click(
            &webview,
            (point.x, point.y),
            (point.viewport_width, point.viewport_height),
            guard.validation,
            guard.sent,
            guard.allowed,
        )
        .await
    }

    /// Native evaluation with a JSON answer (no IPC): the console drain and the pick. A page that
    /// navigates away before answering drops the callback: answered as an empty JSON string value.
    pub(super) async fn evaluate_inner(
        &self,
        id: &MountId,
        script: String,
    ) -> Result<String, EngineError> {
        // A cancelled Linux snapshot must finish restoring native zoom before the capture's page cleanup runs.
        #[cfg(target_os = "linux")]
        let _visibility = self.visibility.lock().await;
        let webview = self.webview(id)?;
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let tx = Mutex::new(Some(tx));
        webview
            .eval_with_callback(script, move |json| {
                if let Some(tx) = tx.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    let _ = tx.send(json);
                }
            })
            .map_err(|e| EngineError::Platform(e.to_string()))?;
        match tokio::time::timeout(DRAIN_WAIT, rx).await {
            Ok(Ok(json)) => Ok(json),
            Ok(Err(_)) => Ok("\"\"".to_owned()),
            Err(_) => Err(EngineError::Timeout("evaluating a page script".to_owned())),
        }
    }
}
