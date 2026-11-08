//! Host Debug API startup after vault unlock and user-only token regeneration.
//! Uses the vault-backed server and UI attribution; called at startup, after vault commands and by Settings.
use tracing::warn;
use vw_core_api::DebugApiFlag;
use vw_debug_api::{DebugApi, DebugApiConfig, DebugApiError};

use crate::Host;

impl Host {
    pub(crate) async fn start_debug_api(&self, user_setting: Option<bool>) {
        let Some(core) = &self.core else { return };
        let mut slot = self.debug.lock().await;
        // A later vault lock does not stop an already authenticated outside tool.
        if slot.is_some() {
            return;
        }
        let mut config = DebugApiConfig::new(self.descriptor_dir.clone());
        config.keys = self.keys.clone();
        config.user_setting = user_setting;
        config.dev_build = self.dev_build;
        config.ui_routes = self.routes.clone();
        config.window = self.app_window.clone();
        match DebugApi::start(core.clone(), config).await {
            Ok(api) => {
                if let (Some(api), Some(guard)) = (&api, core.network_guard()) {
                    guard.add_internal_port(api.port());
                }
                *slot = api;
            }
            Err(error) => {
                core.set_debug_api_flag(DebugApiFlag {
                    enabled: false,
                    reason: "vault-unavailable".to_owned(),
                });
                warn!(%error, "the Debug API did not start");
            }
        }
    }

    /// Returns the existing vault item id for Settings, or creates/regenerates it only on request.
    /// The historical command name is retained; the return value contains no token or token-file path.
    /// Copy uses the ordinary user-only `secrets.copy` command and native expiring clipboard.
    ///
    /// # Errors
    /// A locked/unavailable vault or an operation attributed to a Debug API UI request.
    pub async fn debug_token_file(&self, rotate: bool) -> Result<Option<String>, DebugApiError> {
        if self.ui.driving() {
            return Err(DebugApiError::UserOnly);
        }
        let slot = self.debug.lock().await;
        if let Some(keys) = &self.keys {
            if !rotate {
                // Opening Settings reads metadata only. The Debug API creates its token when enabled.
                return Ok(keys
                    .project_secrets()
                    .await?
                    .iter()
                    .any(|item| item.id == vw_keys::DEBUG_TOKEN_ITEM)
                    .then(|| vw_keys::DEBUG_TOKEN_ITEM.to_owned()));
            }
            if let Some(api) = slot.as_ref() {
                api.rotate_token().await?;
            } else {
                keys.debug_api_token(true).await?;
            }
            return Ok(Some(vw_keys::DEBUG_TOKEN_ITEM.to_owned()));
        }
        // Disposable development hosts without a key service retain the test driver contract.
        #[cfg(debug_assertions)]
        if DebugApiConfig::new(self.descriptor_dir.clone()).test_token_file {
            if let Some(api) = slot.as_ref() {
                if rotate {
                    api.rotate_token().await?;
                }
                if let Some(path) = api.token_path() {
                    return Ok(Some(path.display().to_string()));
                }
            }
            if rotate {
                vw_debug_api::rotate_token_file(&self.descriptor_dir)?;
            }
            return Ok(Some(
                self.descriptor_dir
                    .join(vw_debug_api::TOKEN_FILE)
                    .display()
                    .to_string(),
            ));
        }
        Err(DebugApiError::VaultUnavailable)
    }
}
