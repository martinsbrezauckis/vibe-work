//! Global Developer permissions setting, saved in app data and changed only by a native window action.
//! Depends on `app_settings` and the core's native confirmer; called by Settings and agent tool policy.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::{Map, Value};
use tokio::sync::Mutex;

use crate::command_origin::debug_api_acts;
use crate::{CommandResult, Core, CoreError, ErrorCode, Origin};

const METHOD: &str = "vibe-work/developerPermissions";

/// Current setting plus a revision that invalidates an enable confirmation after an Off request.
pub(crate) struct DeveloperPermissions {
    path: Option<PathBuf>,
    enabled: AtomicBool,
    revision: AtomicU64,
    write: Mutex<()>,
}

impl DeveloperPermissions {
    pub(crate) fn new(data: Option<&Path>) -> Self {
        let path = data.map(|data| data.join("settings.json"));
        let enabled = path
            .as_deref()
            .is_some_and(crate::app_settings::developer_permissions_enabled);
        Self {
            path,
            enabled: AtomicBool::new(enabled),
            revision: AtomicU64::new(0),
            write: Mutex::new(()),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn set_for_test(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    async fn save(&self, enabled: bool, expected_revision: Option<u64>) -> Result<bool, CoreError> {
        let _write = self.write.lock().await;
        if expected_revision
            .is_some_and(|revision| self.revision.load(Ordering::Acquire) != revision)
        {
            return Err(CoreError::new(
                ErrorCode::Denied,
                vw_vocabulary::texts::DEVELOPER_PERMISSIONS_CHANGED,
            ));
        }
        self.write_setting(enabled)?;
        self.enabled.store(enabled, Ordering::Release);
        self.revision.fetch_add(1, Ordering::AcqRel);
        Ok(enabled)
    }

    fn write_setting(&self, enabled: bool) -> Result<(), CoreError> {
        let path = self.path.as_ref().ok_or_else(|| {
            CoreError::new(
                ErrorCode::Internal,
                vw_vocabulary::texts::DEVELOPER_PERMISSIONS_UNAVAILABLE,
            )
        })?;
        crate::write_app_settings(
            path,
            Map::from_iter([("developerPermissions".to_owned(), Value::Bool(enabled))]),
        )
        .map_err(|_| {
            CoreError::new(
                ErrorCode::Internal,
                vw_vocabulary::texts::DEVELOPER_PERMISSIONS_SAVE_FAILED,
            )
        })?;
        Ok(())
    }

    async fn disable(&self) -> Result<bool, CoreError> {
        // Order Off with any enable already saving, as well as with confirmations still open.
        let _write = self.write.lock().await;
        self.revision.fetch_add(1, Ordering::AcqRel);
        if self.enabled() {
            self.write_setting(false)?;
            self.enabled.store(false, Ordering::Release);
        }
        Ok(false)
    }
}

impl Core {
    pub(crate) async fn developer_permissions(
        &self,
        origin: Origin,
        enabled: Option<bool>,
    ) -> Result<CommandResult, CoreError> {
        let state = &self.shared.developer_permissions;
        let Some(enabled) = enabled else {
            return Ok(CommandResult::DeveloperPermissions {
                enabled: state.enabled(),
            });
        };
        if debug_api_acts(&self.shared, origin) {
            return Err(CoreError::new(
                ErrorCode::Denied,
                vw_vocabulary::texts::DEVELOPER_PERMISSIONS_WINDOW_ONLY,
            ));
        }
        if enabled && !state.enabled() {
            let revision = state.revision.load(Ordering::Acquire);
            let summary = vw_vocabulary::texts::DEVELOPER_PERMISSIONS_CONFIRM.to_owned();
            self.ask_native(
                vw_gate::Requester::Window,
                METHOD,
                serde_json::json!({"enabled": true}),
                summary,
            )
            .await?;
            return Ok(CommandResult::DeveloperPermissions {
                enabled: state.save(true, Some(revision)).await?,
            });
        }
        if !enabled {
            return Ok(CommandResult::DeveloperPermissions {
                enabled: state.disable().await?,
            });
        }
        Ok(CommandResult::DeveloperPermissions {
            enabled: state.enabled(),
        })
    }
}
