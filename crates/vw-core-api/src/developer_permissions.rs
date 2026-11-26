//! Global Developer permissions setting, saved in app data and changed only by a native window action.
//! Depends on `app_settings` and the core's native confirmer; called by Settings and agent tool policy.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::{Map, Value};
use tokio::sync::{Mutex, MutexGuard};

use crate::command_origin::debug_api_acts;
use crate::{CommandResult, Core, CoreError, ErrorCode, Origin};

const METHOD: &str = "vibe-work/developerPermissions";

/// Why a forced internal MCP refresh was withheld.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshWithheldCause {
    PausedCommandServer,
    DisabledCommandServer,
    InspectFailed,
}

#[derive(Default)]
struct RefreshNoticeDedupe {
    revision: Option<u64>,
    paused_emitted: bool,
    disabled_emitted: bool,
    inspect_failed_emitted: bool,
}

impl RefreshNoticeDedupe {
    fn take(&mut self, revision: u64, cause: RefreshWithheldCause) -> bool {
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.paused_emitted = false;
            self.disabled_emitted = false;
            self.inspect_failed_emitted = false;
        }
        let slot = match cause {
            RefreshWithheldCause::PausedCommandServer => &mut self.paused_emitted,
            RefreshWithheldCause::DisabledCommandServer => &mut self.disabled_emitted,
            RefreshWithheldCause::InspectFailed => &mut self.inspect_failed_emitted,
        };
        !std::mem::replace(slot, true)
    }
}

/// Current setting plus a revision that invalidates an enable confirmation after an Off request.
pub(crate) struct DeveloperPermissions {
    path: Option<PathBuf>,
    enabled: AtomicBool,
    revision: AtomicU64,
    write: Mutex<()>,
    refresh_notice: std::sync::Mutex<RefreshNoticeDedupe>,
    resumed_commands: std::sync::Mutex<Vec<String>>,
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
            refresh_notice: std::sync::Mutex::new(RefreshNoticeDedupe::default()),
            resumed_commands: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The app's settings file, `None` when the core has no app data folder.
    pub(crate) fn settings_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Names resumed by ON, read under the mutation lock at session apply.
    pub(crate) fn resumed_commands(&self) -> Vec<String> {
        self.resumed_commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// Serializes switch transitions with every app-mediated command-server config mutation.
    pub(crate) async fn mutation_lock(&self) -> MutexGuard<'_, ()> {
        self.write.lock().await
    }

    /// Claims the single visible withheld-refresh notice for the current settings `revision`
    /// and `cause`. Returns `true` once per revision and cause.
    pub(crate) fn claim_refresh_withheld_notice(&self, cause: RefreshWithheldCause) -> bool {
        self.refresh_notice
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take(self.revision(), cause)
    }

    #[cfg(test)]
    pub(crate) fn set_for_test(&self, enabled: bool) {
        if self.enabled.swap(enabled, Ordering::AcqRel) != enabled {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn save_locked(
        &self,
        enabled: bool,
        expected_revision: Option<u64>,
    ) -> Result<bool, CoreError> {
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

    fn disable_locked(&self) -> Result<bool, CoreError> {
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
            let _mutation = state.mutation_lock().await;
            crate::command_pause::refresh_has_command_server(&self.shared.instructions.own_home)
                .map_err(|_| {
                    CoreError::new(
                        ErrorCode::Internal,
                        vw_vocabulary::texts::COMMAND_SERVER_RESUME_FAILED,
                    )
                })?;
            let saved = state.save_locked(true, Some(revision))?;
            self.command_servers_follow(true).await?;
            crate::switch_apply::published(&self.shared);
            return Ok(CommandResult::DeveloperPermissions { enabled: saved });
        }
        if !enabled {
            let _mutation = state.mutation_lock().await;
            crate::project_mcp::before_off(&self.shared).await?;
            // Pause on disk before publishing OFF: a concurrent session open can only see the already-disabled
            // config once the switch reports OFF. Open chats apply it before their next work (`switch_apply.rs`).
            self.command_servers_follow(false).await?;
            let saved = state.disable_locked()?;
            crate::switch_apply::published(&self.shared);
            return Ok(CommandResult::DeveloperPermissions { enabled: saved });
        }
        Ok(CommandResult::DeveloperPermissions {
            enabled: state.enabled(),
        })
    }

    /// Command-started MCP servers follow the switch (`command_pause`): off pauses them, on resumes the ones the app
    /// paused. Mark open chats stale; after publication idle chats reload in the background and running chats
    /// apply at turn end. Before-work checks retain recovery/refusal for a failed apply (`switch_apply.rs`).
    async fn command_servers_follow(&self, on: bool) -> Result<(), CoreError> {
        let home = self.shared.instructions.own_home.clone();
        let settings = self
            .shared
            .developer_permissions
            .settings_path()
            .map(Path::to_path_buf);
        let (changed, resumed) = crate::worktrees::blocking(move || {
            // resume_paused clears this list. Keep its names for targeted session convergence, never a
            // broad discovery (which would run rows the person disabled in pinned Vibe 3.1.4).
            let resumed = if on {
                settings
                    .as_deref()
                    .map(crate::app_settings::paused_command_servers)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let changed = if on {
                crate::command_pause::resume_paused(&home, settings.as_deref())
            } else {
                crate::command_pause::pause_all_checked(&home, settings.as_deref())
            }?;
            Ok::<_, crate::mcp_keys_config::ConfigEditError>((changed, resumed))
        })
        .await?
        .map_err(|_| {
            CoreError::new(
                ErrorCode::Internal,
                if on {
                    vw_vocabulary::texts::COMMAND_SERVER_RESUME_FAILED
                } else {
                    vw_vocabulary::texts::COMMAND_SERVER_PAUSE_FAILED
                },
            )
        })?;
        *self
            .shared
            .developer_permissions
            .resumed_commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = resumed;
        if changed > 0 {
            self.shared.channels.bump_switch_generation();
        }
        Ok(())
    }
}
