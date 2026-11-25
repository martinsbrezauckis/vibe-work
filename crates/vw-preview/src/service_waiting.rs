//! Admission of chats that wait for a built-in browser slot: when capacity frees (a tab or chat closes, an idle
//! engine is released, the cap is raised), the oldest waiting chat gets the slot and its pending page opens.
//! Depends on the pool admission state and the mount boundary; started by the operations that free a slot.
//! Lock order: a waiter's session lock, then `pool_op`; the pool and preview state locks are never held across
//! an await. An admission pass holds no lock of its own while it waits, so it never joins a lock cycle.

use super::PreviewService;
use crate::layout;
use crate::state::BrowserMountState;

/// What one waiter's turn at a free slot came to.
enum Step {
    /// The queue changed or the waiter was dropped from it: look at the head again.
    Again,
    /// The waiter's page was mounted (or refused by the engine): finish its follow-up work.
    Opened,
    /// No slot is free (any more), or the app is quitting.
    Stop,
}

impl PreviewService {
    /// Whether any chat waits for a free browser slot.
    #[must_use]
    pub fn has_waiting_browsers(&self) -> bool {
        !self.lock_pool().waiting.is_empty()
    }

    /// Starts an admission pass in the background unless one is already scheduled (it then runs once more).
    /// Used where the caller holds session or pool locks and so cannot run the pass itself.
    pub(super) fn schedule_admission(&self) {
        {
            let mut pool = self.lock_pool();
            if pool.waiting.is_empty() {
                return;
            }
            if pool.admission_running {
                pool.admission_again = true;
                return;
            }
            pool.admission_running = true;
        }
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                service.admit_waiting().await;
                let mut pool = service.lock_pool();
                if pool.admission_again {
                    pool.admission_again = false;
                    continue;
                }
                pool.admission_running = false;
                return;
            }
        });
    }

    /// Gives every free slot to the oldest waiting chats, one after the other, and opens their pending pages.
    /// A waiter whose chat or tab closed, whose wait was cancelled, or whose page the engine refuses is skipped:
    /// its slot goes to the next one. Never admits beyond the current cap. Callers hold no session lock.
    pub async fn admit_waiting(&self) {
        while let Some(waiter) = self.next_waiter() {
            let _activity = self.session_lock(&waiter).await;
            let step = {
                let _operation = self.pool_op.lock().await;
                self.admit_one(&waiter).await
            };
            match step {
                Step::Stop => return,
                Step::Again => {}
                Step::Opened => {
                    self.refresh_rules().await;
                    self.enforce_cap(&waiter).await;
                    self.inner.changed(&waiter);
                }
            }
        }
    }

    /// The oldest waiter while a slot is free.
    fn next_waiter(&self) -> Option<String> {
        let pool = self.lock_pool();
        if pool.shutting_down {
            return None;
        }
        let waiter = pool.waiting.front()?;
        (pool.active.len() < pool.snapshot().effective_cap as usize).then(|| waiter.clone())
    }

    /// Registers the still-waiting FIFO head, or leaves the next admission pass to inspect the queue again.
    fn register_waiter(&self, waiter: &str) -> bool {
        let mut pool = self.lock_pool();
        // Stop removes the FIFO entry under this lock. Once registered, a later Stop
        // records withdrawal until the native mount is reconciled below.
        if pool.waiting.front().map(String::as_str) != Some(waiter) {
            return false;
        }
        if !self
            .inner
            .lock()
            .entries
            .get(waiter)
            .is_some_and(|entry| entry.browser_mount == BrowserMountState::Waiting)
        {
            pool.remove_waiting(waiter);
            return false;
        }
        pool.admitting_waiters.insert(waiter.to_owned());
        if pool.withdrawn_waiters.remove(waiter) {
            pool.admitting_waiters.remove(waiter);
            return false;
        }
        true
    }

    /// Caller owns the waiter's session lock and `pool_op`; everything is checked again here.
    async fn admit_one(&self, waiter: &str) -> Step {
        {
            let pool = self.lock_pool();
            if pool.shutting_down || pool.active.len() >= pool.snapshot().effective_cap as usize {
                return Step::Stop;
            }
            if pool.waiting.front().map(String::as_str) != Some(waiter) {
                return Step::Again;
            }
        }
        let job = {
            let state = self.inner.lock();
            state.entries.get(waiter).and_then(|entry| {
                let eligible = entry.browser_mount == BrowserMountState::Waiting
                    && entry.mount.is_none()
                    && entry.server_status.is_none();
                let url = entry.url.clone().or_else(|| entry.pending_open.clone())?;
                let placement = state
                    .shown
                    .clone()
                    .filter(|shown| shown.session_id == waiter)
                    .and_then(|shown| {
                        layout::place_viewport(shown.area, entry.width, entry.viewport).ok()
                    });
                eligible.then_some((url, placement))
            })
        };
        let Some((url, placement)) = job else {
            // Closed or replaced while it waited: nothing to open.
            self.lock_pool().remove_waiting(waiter);
            if let Some(entry) = self.inner.lock().entries.get_mut(waiter)
                && entry.browser_mount == BrowserMountState::Waiting
            {
                entry.browser_mount = BrowserMountState::Ready;
            }
            self.inner.changed(waiter);
            return Step::Again;
        };
        #[cfg(feature = "e2e-script")]
        if let Some(pause) = &self.config.test_before_wait_registration {
            pause().await;
        }
        if !self.register_waiter(waiter) {
            return Step::Again;
        }
        let mounted = self.mount_locked(waiter, url.clone(), placement).await;
        // Keep the waiter cancellable until its state is ready to publish. A Stop that
        // arrives during this transition still marks it withdrawn below.
        if mounted.is_ok()
            && let Some(entry) = self.inner.lock().entries.get_mut(waiter)
        {
            entry.browser_mount = BrowserMountState::Ready;
            if let Some(mount) = entry.mount.as_mut() {
                mount.state = crate::state::MountState::Ready;
            }
        }
        let withdrawn = {
            let mut pool = self.lock_pool();
            pool.admitting_waiters.remove(waiter);
            pool.withdrawn_waiters.remove(waiter)
        };
        if withdrawn {
            if let Some(entry) = self.inner.lock().entries.get_mut(waiter) {
                entry.browser_mount = BrowserMountState::Withdrawn;
            }
            if let Err(error) = self.unmount_in_pool(waiter).await {
                // A failed native close retains its slot for a later retry; revoke its callbacks now.
                tracing::warn!(session = %waiter, %error, "withdrawn browser mount could not close");
                let label = self
                    .inner
                    .lock()
                    .entries
                    .get(waiter)
                    .and_then(|entry| entry.mount.as_ref().map(|mount| mount.label.clone()));
                if let Some(label) = label {
                    self.revoke_failed_mount(waiter, &label);
                }
            }
            if let Some(entry) = self.inner.lock().entries.get_mut(waiter) {
                entry.pending_open = None;
                entry.set_loading(false);
            }
            self.inner.changed(waiter);
            return Step::Again;
        }
        match mounted {
            Ok(()) => {}
            Err(error) => {
                tracing::warn!(session = %waiter, %error, "waiting browser page could not open");
                if let Some(entry) = self.inner.lock().entries.get_mut(waiter) {
                    entry.browser_mount = BrowserMountState::Ready;
                    entry.note_refused(
                        url.as_str(),
                        &format!("the page could not open once the browser was free: {error}"),
                    );
                }
            }
        }
        Step::Opened
    }

    /// The user's chat was stopped: its waiting request is withdrawn, so a slot that frees later goes to the
    /// next chat. The page keeps its address and opens again on a new request or explicit tab selection.
    /// Returns whether a queued or in-flight waiting page was withdrawn; cancellation is the primary effect.
    #[allow(clippy::must_use_candidate)]
    pub fn cancel_browser_wait(&self, session_id: &str) -> bool {
        let (queued, admitting) = {
            let mut pool = self.lock_pool();
            let queued = pool.waiting.iter().any(|queued| queued == session_id);
            pool.remove_waiting(session_id);
            let admitting = pool.admitting_waiters.contains(session_id);
            if admitting {
                pool.withdrawn_waiters.insert(session_id.to_owned());
            }
            (queued, admitting)
        };
        if !queued {
            return admitting;
        }
        if let Some(entry) = self.inner.lock().entries.get_mut(session_id)
            && entry.browser_mount == BrowserMountState::Waiting
        {
            entry.browser_mount = BrowserMountState::Withdrawn;
            entry.pending_open = None;
            entry.pending_started = false;
            entry.set_loading(false);
        }
        self.inner.changed(session_id);
        true
    }
}
