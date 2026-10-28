//! Native close and confirmed-close bookkeeping for the preview engine.
//! Depends on Tauri and guard membership; called by ordinary close and failed mount setup.

use std::sync::PoisonError;
use tauri::{Webview, Wry};
use vw_preview::{EngineError, MountId};

use super::TauriEngine;
#[cfg(target_os = "macos")]
use super::mac_guard::own_store_key;

impl TauriEngine {
    pub(super) fn close_webview(webview: &Webview<Wry>) -> Result<(), EngineError> {
        // A scratch marker lets native fixtures retain a child after setup refusal in debug builds only.
        if cfg!(debug_assertions)
            && std::env::var_os("VW_DEV_PREVIEW_CLOSE_FAULT")
                .is_some_and(|path| std::path::Path::new(&path).exists())
        {
            return Err(EngineError::Platform(
                "injected native close failure".to_owned(),
            ));
        }
        webview
            .close()
            .map_err(|e| EngineError::Platform(e.to_string()))
    }

    pub(super) fn close_inner(&self, id: &MountId) -> Result<(), EngineError> {
        match self.webview(id) {
            Ok(webview) => Self::close_webview(&webview)?,
            Err(EngineError::NotMounted) => {}
            Err(other) => return Err(other),
        }
        // A ready child whose close failed must keep its protection and remain usable on retry.
        if let Some(guard) = &self.guard {
            guard.forget_session(&id.label);
            #[cfg(target_os = "macos")]
            guard.forget_store(&own_store_key(&id.label));
        }
        self.drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id.label);
        self.context_pollers.stop(&id.label);
        self.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id.label);
        #[cfg(target_os = "macos")]
        crate::rules_macos::forget(&self.app, &id.label, &self.rule_generations);
        Ok(())
    }
}
