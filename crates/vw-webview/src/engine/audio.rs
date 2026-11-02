//! Native silence across all tabs owned by this engine, including hidden tabs and cancellation.
//! The mount lock excludes new pages until restoration finishes. Page JavaScript cannot undo native silence.
use super::TauriEngine;
use crate::platform_audio;
use tauri::Manager;
use vw_preview::{AudioQuiet, EngineError};

impl TauriEngine {
    pub(super) async fn quiet_audio_inner(&self) -> Result<AudioQuiet, EngineError> {
        let held = self.audio_lock.clone().lock_owned().await;
        let labels: Vec<_> = self
            .drains
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        let app = self.app.clone();
        let window = self.window_label.clone();
        let (end, wait) = tokio::sync::oneshot::channel::<()>();
        let lease = AudioQuiet::new(move || drop(end));
        let (ready, started) = tokio::sync::oneshot::channel();
        // A dropped command must still restore every native change it scheduled.
        tokio::spawn(async move {
            let _held = held;
            let mut saved = Vec::new();
            let mut result = Ok(());
            for label in labels {
                let Some(view) = app.get_webview(&label) else {
                    continue;
                };
                let previous = std::sync::Arc::new(std::sync::Mutex::new(None));
                saved.push((label, previous.clone()));
                match platform_audio::quiet_audio(&view, previous, true).await {
                    Ok(()) => {}
                    Err(error) => {
                        result = Err(error);
                        break;
                    }
                }
            }
            // Close removes bookkeeping after queuing the native close. Flush that queue even when no tabs remain.
            if result.is_ok() {
                result = match app.get_webview(&window) {
                    Some(main) => platform_audio::barrier(&main).await,
                    None => Err(EngineError::NotMounted),
                };
            }
            let succeeded = result.is_ok();
            let _ = ready.send(result);
            if succeeded {
                let _ = wait.await;
            }
            for (label, previous) in saved {
                if let Some(view) = app.get_webview(&label)
                    && let Err(error) = platform_audio::quiet_audio(&view, previous, false).await
                {
                    tracing::warn!(%error, "browser audio restoration failed");
                }
            }
        });
        started
            .await
            .map_err(|_| EngineError::Platform("Browser audio could not be silenced".into()))??;
        Ok(lease)
    }
}
