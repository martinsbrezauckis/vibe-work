//! External browser pool settings, snapshot and release operations for the preview service.
//! Depends on the pool state and service mount boundary; called by the core and periodic reaper.

use super::{PreviewError, PreviewService};
use crate::engine_pool::{IDLE_TIMEOUT, PoolState, setting_path, valid_setting};
use crate::layout::Rect;
use crate::state::BrowserMountState;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static SETTINGS_NONCE: AtomicU64 = AtomicU64::new(0);

fn save_setting(path: &std::path::Path, data: &[u8]) -> Result<(), PreviewError> {
    let nonce = SETTINGS_NONCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("json.tmp-{}-{nonce}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|e| PreviewError::PoolStorage(e.to_string()))?;
    let written = file.write_all(data).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(PreviewError::PoolStorage(error.to_string()));
    }
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        PreviewError::PoolStorage(error.to_string())
    })
}

impl PreviewService {
    pub(crate) fn touch_pool(&self, session_id: &str) {
        if let Some(used) = self.lock_pool().active.get_mut(session_id) {
            *used = tokio::time::Instant::now();
        }
    }
    pub(super) fn lock_pool(&self) -> std::sync::MutexGuard<'_, PoolState> {
        self.pool
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The configured and currently admitted external browser sessions.
    #[must_use]
    pub fn engine_pool(&self) -> crate::EnginePoolSnapshot {
        self.lock_pool().snapshot()
    }

    /// Saves `auto` or a fixed limit from `1` through `4`, then returns the new snapshot.
    ///
    /// # Errors
    /// Invalid input or a failed settings write.
    pub async fn set_engine_pool(
        &self,
        setting: &str,
    ) -> Result<crate::EnginePoolSnapshot, PreviewError> {
        let setting = setting.trim().to_ascii_lowercase();
        if !valid_setting(&setting) {
            return Err(PreviewError::PoolSetting(
                "expected auto or 1, 2, 3, 4".to_owned(),
            ));
        }
        let operation = self.pool_op.lock().await;
        std::fs::create_dir_all(&self.config.storage_root)
            .map_err(|e| PreviewError::PoolStorage(e.to_string()))?;
        let path = setting_path(&self.config.storage_root);
        let data =
            serde_json::to_vec(&setting).map_err(|e| PreviewError::PoolStorage(e.to_string()))?;
        save_setting(&path, &data)?;
        self.lock_pool().configured = setting;
        drop(operation);
        Ok(self.engine_pool())
    }

    /// Protects a choice from its initial capture through the name prompt and native confirmation.
    pub async fn browser_save_guard(&self, session_id: &str) -> crate::BrowserSaveGuard {
        // Serialize capture admission with a victim's close before accepting a user choice.
        let _activity = self.session_lock(session_id).await;
        let _operation = self.pool_op.lock().await;
        *self
            .lock_pool()
            .saves
            .entry(session_id.to_owned())
            .or_default() += 1;
        crate::BrowserSaveGuard {
            pool: self.pool.clone(),
            session_id: session_id.to_owned(),
        }
    }

    /// Records which Preview panel the user selected and places or parks its native view.
    /// A selected panel stays protected while a modal temporarily removes its placement.
    ///
    /// # Errors
    /// Invalid placement or an engine failure while changing the native view.
    pub async fn layout_panel(
        &self,
        session_id: Option<String>,
        rect: Option<Rect>,
    ) -> Result<(), PreviewError> {
        self.layout_selected(
            session_id.clone().zip(rect),
            super::service_layout::PanelSelection::Set(session_id),
        )
        .await
    }

    pub(super) async fn select_browser_panel(&self, session_id: Option<String>) {
        let _operation = self.pool_op.lock().await;
        let mut pool = self.lock_pool();
        if pool.selected_panel != session_id {
            if let Some(old) = pool.selected_panel.take()
                && let Some(used) = pool.active.get_mut(&old)
            {
                *used = tokio::time::Instant::now();
            }
            pool.selected_panel = session_id;
        }
    }

    pub(super) fn browser_protected(&self, session: &str) -> bool {
        let (owner, agent_current) = {
            let state = self.inner.lock();
            let owner = state.owner_of(session);
            let agent_current = state
                .tabs
                .get(&owner)
                .is_some_and(|tabs| tabs.agent.as_deref() == Some(session));
            (owner, agent_current)
        };
        let pool = self.lock_pool();
        let turn_protected = agent_current
            && (pool.active_turns.contains(&owner)
                || pool.waiting_turns.contains(&owner)
                || pool.pending_turn_starts.contains(&owner));
        let protected = turn_protected
            || pool.saves.contains_key(session)
            || pool.selected_panel.as_deref() == Some(session);
        drop(pool);
        protected
            || self
                .inner
                .lock()
                .shown
                .as_ref()
                .is_some_and(|shown| shown.session_id == session)
    }

    pub(super) fn browser_released(&self, session: &str) {
        if let Some(entry) = self.inner.lock().entries.get_mut(session) {
            entry.browser_mount = BrowserMountState::Released;
            entry.loading = false;
        }
        self.inner.changed(session);
    }

    pub(super) fn forget_browser_turn(&self, session: &str) {
        let mut pool = self.lock_pool();
        pool.turn_epochs.remove(session);
        pool.pending_turn_starts.remove(session);
        pool.active_turns.remove(session);
        pool.waiting_turns.remove(session);
        if pool.selected_panel.as_deref() == Some(session) {
            pool.selected_panel = None;
        }
        // Save counts are owned by guards, including any native confirmation still unwinding.
    }

    /// True if the latest external mount request could not get a slot.
    #[must_use]
    pub fn browser_waiting(&self, session_id: &str) -> bool {
        self.lock_pool().waiting.contains(session_id)
    }

    /// Marks a newly started turn before its session state is observed.
    pub fn browser_turn_started(&self, session_id: &str) {
        let mut pool = self.lock_pool();
        let epoch = pool.turn_epochs.entry(session_id.to_owned()).or_default();
        *epoch = epoch.wrapping_add(1);
        pool.pending_turn_starts.insert(session_id.to_owned());
        pool.active_turns.insert(session_id.to_owned());
    }

    /// Marks the new turn's active session state as observed.
    pub fn browser_turn_observed(&self, session_id: &str) {
        self.lock_pool().pending_turn_starts.remove(session_id);
    }

    /// Invalidates older releases when an active turn arrives, including queued turns.
    pub fn browser_turn_active(&self, session_id: &str) {
        self.browser_turn_started(session_id);
        self.browser_turn_observed(session_id);
    }

    /// Protects a session until its outstanding user callbacks have been answered.
    pub fn browser_turn_waiting(&self, session_id: &str, waiting: bool) {
        let mut pool = self.lock_pool();
        if waiting {
            pool.waiting_turns.insert(session_id.to_owned());
        } else {
            pool.waiting_turns.remove(session_id);
        }
    }

    /// Current turn generation, or `None` while a new turn start is pending.
    #[must_use]
    pub fn browser_turn_epoch(&self, session_id: &str) -> Option<u64> {
        let pool = self.lock_pool();
        (!pool.pending_turn_starts.contains(session_id))
            .then(|| pool.turn_epochs.get(session_id).copied().unwrap_or(0))
    }

    /// Ends protection for a completed turn without closing its document or hiding its panel.
    /// A newer turn invalidates this completion; a denied turn keeps its waiting request.
    ///
    /// # Errors
    /// Reserved for preview service errors; completion itself makes no engine calls.
    pub async fn release_browser_for_turn(
        &self,
        session_id: &str,
        epoch: u64,
    ) -> Result<bool, PreviewError> {
        let _activity = self.session_lock(session_id).await;
        let mut pool = self.lock_pool();
        if pool.pending_turn_starts.contains(session_id)
            || pool.turn_epochs.get(session_id).copied().unwrap_or(0) != epoch
        {
            return Ok(false);
        }
        pool.active_turns.remove(session_id);
        if let Some(used) = pool.active.get_mut(session_id) {
            *used = tokio::time::Instant::now();
        }
        Ok(true)
    }

    /// Parks a browser view while preserving its document and slot.
    ///
    /// # Errors
    /// An engine close failure.
    pub async fn release_browser(&self, session_id: &str) -> Result<(), PreviewError> {
        let _activity = self.session_lock(session_id).await;
        let mount = self
            .inner
            .lock()
            .entries
            .get(session_id)
            .and_then(|entry| entry.ready_mount().cloned());
        if let Some(mount) = mount {
            self.bounded(
                "park",
                self.engine.place(&self.mount_id(session_id, &mount), None),
            )
            .await?;
        }
        self.touch_pool(session_id);
        Ok(())
    }

    /// Closes only hidden, unprotected engines idle for ten minutes.
    /// Memory/setting cap changes apply to new admissions, never proactive teardown.
    ///
    /// # Errors
    /// The first engine close failure.
    pub async fn reap_idle_browsers(&self) -> Result<(), PreviewError> {
        let expired: Vec<String> = self
            .lock_pool()
            .active
            .iter()
            .filter(|(_, used)| used.elapsed() >= IDLE_TIMEOUT)
            .map(|(session, _)| session.clone())
            .collect();
        for session in expired {
            let Some(_activity) = self.try_session_lock(&session) else {
                continue;
            };
            let _operation = self.pool_op.lock().await;
            let still_idle = self
                .lock_pool()
                .active
                .get(&session)
                .is_some_and(|used| used.elapsed() >= IDLE_TIMEOUT);
            if still_idle && !self.browser_protected(&session) {
                self.suspend_in_pool(&session).await?;
                self.browser_released(&session);
            }
        }
        Ok(())
    }
}
