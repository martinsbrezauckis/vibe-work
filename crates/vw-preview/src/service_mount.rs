//! Engine mount, unmount and suspended-preview restoration for the preview service.
//! Depends on URL classification, the pool admission state and engine calls; called by service operations.

use super::{PreviewError, PreviewService};
use crate::console;
use crate::engine::{EngineError, MountId, MountSpec};
use crate::layout::{self, Placement};
use crate::state::{BrowserMountState, Mount};
use url::Url;

fn initialization_script(accessor: &str, page: &str) -> String {
    format!(
        "{}\n{}",
        console::init_script(accessor),
        crate::page_init::page_init_script(page)
    )
}

impl PreviewService {
    pub(crate) fn mount_id(&self, session_id: &str, mount: &Mount) -> MountId {
        let _ = self;
        MountId {
            session_id: session_id.to_owned(),
            label: mount.label.clone(),
        }
    }

    /// Reserves a slot before any normal Browser page engine is created.
    async fn admit_browser(&self, session_id: &str) -> Result<bool, PreviewError> {
        // The App preview and every web tab compete for the same native engine cap.
        // NUL-prefixed internal checker/account mounts are outside the Browser panel.
        let external = !session_id.starts_with('\0');
        if external {
            self.make_browser_room(session_id).await;
            let mut pool = self.lock_pool();
            if !pool.active.contains_key(session_id)
                && pool.active.len() >= pool.snapshot().effective_cap as usize
            {
                pool.waiting.insert(session_id.to_owned());
                drop(pool);
                if let Some(entry) = self.inner.lock().entries.get_mut(session_id) {
                    entry.browser_mount = BrowserMountState::Waiting;
                    entry.loading = false;
                }
                self.inner.changed(session_id);
                return Err(PreviewError::BrowserBusy);
            }
            pool.active
                .insert(session_id.to_owned(), tokio::time::Instant::now());
            pool.waiting.remove(session_id);
            if let Some(entry) = self.inner.lock().entries.get_mut(session_id) {
                entry.browser_mount = BrowserMountState::Ready;
            }
        } else {
            self.lock_pool().waiting.remove(session_id);
            if let Some(entry) = self.inner.lock().entries.get_mut(session_id) {
                entry.browser_mount = BrowserMountState::Ready;
            }
        }
        Ok(external)
    }

    /// Called only under `pool_op`: demand evicts idle parked engines in LRU order.
    async fn make_browser_room(&self, requester: &str) {
        let (admitted, cap, mut candidates) = {
            let pool = self.lock_pool();
            (
                pool.active.contains_key(requester),
                pool.snapshot().effective_cap as usize,
                pool.active
                    .iter()
                    .map(|(id, used)| (id.clone(), *used))
                    .collect::<Vec<_>>(),
            )
        };
        if admitted {
            return;
        }
        candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        for (session, _) in candidates {
            if self.lock_pool().active.len() < cap {
                break;
            }
            let Some(_activity) = self.try_session_lock(&session) else {
                continue;
            };
            if self.browser_protected(&session) {
                continue;
            }
            if self.unmount_in_pool(&session).await.is_ok() {
                self.browser_released(&session);
            }
        }
    }

    /// Mounts the session's preview at `url` (hidden unless `placement`).
    pub(super) async fn mount(
        &self,
        session_id: &str,
        url: Url,
        placement: Option<Placement>,
    ) -> Result<(), PreviewError> {
        let _operation = self.pool_op.lock().await;
        let external = self.admit_browser(session_id).await?;
        let session_storage = self.storage(session_id);
        let accessor = console::accessor_name();
        let page = crate::page_init::page_accessor(&accessor);
        let internal = self.inner.internal_ports();
        let (label, parked, rules, class) = {
            let mut state = self.inner.lock();
            state.generation += 1;
            let last_area = state.last_area;
            let label = session_storage.key.label(state.generation);
            state.by_label.insert(label.clone(), session_id.to_owned());
            let entry = state.entry(session_id);
            // The entry's Lan host (set only by the user's open) makes a Lan mount; else the URL decides.
            let class = if entry.lan_host.is_some() {
                crate::policy::PageClass::Lan
            } else {
                self.inner.policy.classify(&url).map_or(
                    crate::policy::PageClass::Web,
                    crate::policy::Destination::class,
                )
            };
            entry.mount = Some(Mount {
                label: label.clone(),
                page: page.clone(),
                class,
                names_later: false,
                rules: None,
                rules_pending: true,
                rules_offered: None,
            });
            // Proven again by every mount, reported again by every mount.
            entry.guarded = false;
            entry.layers.clear();
            let rules = state.rule_page(session_id, class, internal.as_deref());
            if let Some(mount) = state.entry(session_id).mount.as_mut() {
                mount.rules_offered = Some(rules.clone());
            }
            // hidden, it still lays out at the size it would have if shown now.
            let parked = layout::parked(last_area, state.entry(session_id).width);
            (label, parked, rules, class)
        };
        #[cfg(feature = "e2e-script")]
        let fixture_web = self
            .config
            .test_browser_origin
            .as_ref()
            .is_some_and(|origin| origin.origin() == url.origin());
        #[cfg(not(feature = "e2e-script"))]
        let fixture_web = false;
        let storage = if class == crate::policy::PageClass::Web || fixture_web {
            self.shared_web_storage()
        } else {
            session_storage
        };
        let spec = MountSpec {
            id: MountId {
                session_id: session_id.to_owned(),
                label: label.clone(),
            },
            url,
            storage,
            // The console and page environment start on every document.
            console_script: initialization_script(&accessor, &page),
            console_drain: console::drain_script(&accessor),
            context_menu: true,
            placement,
            parked: Some(parked),
            rules: Some(rules),
        };
        if let Err(error) = self.bounded("mount", self.engine.mount(spec)).await {
            let id = MountId {
                session_id: session_id.to_owned(),
                label: label.clone(),
            };
            let closed = matches!(
                self.bounded("close", self.engine.close(&id)).await,
                Ok(()) | Err(EngineError::NotMounted)
            );
            if closed {
                if external {
                    self.lock_pool().active.remove(session_id);
                }
                let mut state = self.inner.lock();
                state.by_label.remove(&label);
                if let Some(entry) = state.entries.get_mut(session_id) {
                    entry.mount = None;
                }
            }
            if let Some(entry) = self.inner.lock().entries.get_mut(session_id) {
                entry.loading = false;
            }
            return Err(error.into());
        }
        Ok(())
    }

    /// Suspends least-recently-shown previews over the live cap (never `keep` or the shown one).
    pub(super) async fn enforce_cap(&self, keep: &str) {
        let excess = self.inner.lock().over_cap(self.config.live_cap, keep);
        for session in excess {
            // The external pool owns these documents and its stronger protection rules.
            if self.lock_pool().active.contains_key(&session) || self.browser_protected(&session) {
                continue;
            }
            let Some(_activity) = self.try_session_lock(&session) else {
                continue;
            };
            if self
                .inner
                .lock()
                .over_cap(self.config.live_cap, keep)
                .contains(&session)
            {
                let _ = self.unmount(&session).await;
            }
        }
    }

    /// Closes the mount of a session's preview, keeping its URL (suspended).
    pub(crate) async fn unmount(&self, session_id: &str) -> Result<(), PreviewError> {
        let _operation = self.pool_op.lock().await;
        self.unmount_in_pool(session_id).await
    }

    /// Caller owns `pool_op` and the session operation lock, including during failed-close recovery.
    pub(super) async fn unmount_in_pool(&self, session_id: &str) -> Result<(), PreviewError> {
        let mount = {
            self.inner
                .lock()
                .entries
                .get(session_id)
                .and_then(|e| e.mount.clone())
        };
        if let Some(mount) = mount {
            let id = self.mount_id(session_id, &mount);
            self.bounded("close", self.engine.close(&id)).await?;
            let mut state = self.inner.lock();
            state.by_label.remove(&mount.label);
            state.browser_overlays.remove(session_id);
            state.browser_overlay_prepared.remove(session_id);
            if let Some(entry) = state.entries.get_mut(session_id) {
                entry.mount = None;
            }
        }
        self.lock_pool().active.remove(session_id);
        self.lock_pool().waiting.remove(session_id);
        if let Some(entry) = self.inner.lock().entries.get_mut(session_id) {
            entry.browser_mount = BrowserMountState::Ready;
        }
        Ok(())
    }

    /// The mount of a session, mounting a suspended preview again (hidden) when it has a URL.
    pub(super) async fn ensure_mounted(&self, session_id: &str) -> Result<MountId, PreviewError> {
        let (mount, url) = {
            let state = self.inner.lock();
            let entry = state
                .entries
                .get(session_id)
                .ok_or(PreviewError::NothingOpen)?;
            (entry.mount.clone(), entry.url.clone())
        };
        if let Some(mount) = mount {
            self.touch_pool(session_id);
            return Ok(self.mount_id(session_id, &mount));
        }
        let url = url.ok_or(PreviewError::NothingOpen)?;
        let (before, mut loads) = {
            let state = self.inner.lock();
            let loads = state
                .entries
                .get(session_id)
                .map(|e| e.loads.subscribe())
                .ok_or(PreviewError::NothingOpen)?;
            let before = *loads.borrow();
            (before, loads)
        };
        self.mount(session_id, url, None).await?;
        self.enforce_cap(session_id).await;
        let _ = tokio::time::timeout(self.config.load_wait, loads.wait_for(|n| *n > before)).await;
        let state = self.inner.lock();
        let mount = state
            .entries
            .get(session_id)
            .and_then(|e| e.mount.clone())
            .ok_or(PreviewError::NothingOpen)?;
        Ok(self.mount_id(session_id, &mount))
    }
}
