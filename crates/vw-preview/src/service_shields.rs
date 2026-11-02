//! Persists confirmed shield settings and updates live mounts under the mount admission lock.
//! Depends on the native engine filter API; called by core Browser settings after list preparation.
use super::{PreviewError, PreviewService, service_pool::save_setting};
use crate::shields::{MAX_LIST_BYTES, Mode, Saved, Settings};
use std::sync::PoisonError;

impl PreviewService {
    /// Confirmed mode and platform support.
    #[must_use]
    pub fn shields(&self) -> Settings {
        Settings {
            mode: self
                .inner
                .shields
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .mode,
            supported: self.engine.supports_content_filter(),
        }
    }

    /// Install compiled rules on every live mount, then persist the selection for future mounts.
    /// Failed compilation or persistence restores the previously confirmed filter.
    ///
    /// # Errors
    /// Unsupported platform, oversized rules, native compile failure, or failed settings write.
    pub async fn set_shields(&self, mode: Mode, rules: String) -> Result<Settings, PreviewError> {
        if !self.engine.supports_content_filter() || rules.len() > MAX_LIST_BYTES {
            return Err(PreviewError::PoolSetting(
                "Ad blocking is unavailable or its rules are too large".into(),
            ));
        }
        let _operation = self.pool_op.lock().await;
        let old = self
            .inner
            .shields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mounts: Vec<_> = self
            .inner
            .lock()
            .entries
            .iter()
            .filter_map(|(key, entry)| entry.mount.as_ref().map(|mount| self.mount_id(key, mount)))
            .collect();
        let saved = Saved { mode, rules };
        let result = async {
            for mount in &mounts {
                self.engine
                    .update_content_filter(mount, saved.rules.clone())
                    .await?;
            }
            std::fs::create_dir_all(&self.config.storage_root)
                .map_err(|error| PreviewError::PoolStorage(error.to_string()))?;
            let data = serde_json::to_vec(&saved)
                .map_err(|error| PreviewError::PoolStorage(error.to_string()))?;
            save_setting(
                &self.config.storage_root.join("browser-shields.json"),
                &data,
            )
        }
        .await;
        if let Err(error) = result {
            for mount in &mounts {
                if let Err(restore) = self
                    .engine
                    .update_content_filter(mount, old.rules.clone())
                    .await
                {
                    tracing::warn!(%restore, "could not restore a browser ad filter");
                }
            }
            return Err(error);
        }
        *self
            .inner
            .shields
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = saved;
        Ok(self.shields())
    }
}
