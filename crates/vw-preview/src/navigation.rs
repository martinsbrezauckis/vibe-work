//! Held personal navigations: bounded DNS completion and per-mount cancellation through UI dispatch.
//! Depends on hooks, registry and the engine continuation contract; called by hooks and service operations.
//! No worker may reopen a URL without the final UI-thread policy and identity check.

use std::sync::Arc;

use url::Url;

use crate::engine::MountId;
use crate::hooks::{Hooks, Inner, Pre};
use crate::service::PreviewService;
use crate::state::Entry;

/// Distinct even for two attempts at the same URL. Mount labels also change on every remount.
#[derive(Clone, Debug, Default)]
pub(crate) struct Navigation {
    generation: u64,
    pub(crate) pending: Option<Url>,
}

impl Entry {
    pub(crate) fn navigation_held(&self) -> bool {
        self.mount
            .as_ref()
            .is_some_and(|m| m.navigation.pending.is_some())
    }

    pub(crate) fn cancel_navigation(&mut self) {
        if let Some(mount) = &mut self.mount {
            mount.navigation.generation = mount.navigation.generation.wrapping_add(1);
            if mount.navigation.pending.take().is_some() {
                self.loading = false;
            }
        }
    }
}

struct AgentNavigation {
    inner: Arc<Inner>,
    session: String,
}

impl Drop for AgentNavigation {
    fn drop(&mut self) {
        if let Some(entry) = self.inner.lock().entries.get_mut(&self.session) {
            entry.agent_navigation = entry.agent_navigation.saturating_sub(1);
        }
    }
}

impl PreviewService {
    /// Admission of a newer command cancels DNS before waiting for the previous command's load lock.
    pub(crate) fn cancel_pending_navigation(&self, session: &str) {
        let mut state = self.inner.lock();
        if let Some(entry) = state.entries.get_mut(session) {
            entry.cancel_navigation();
        }
        drop(state);
        self.inner.changed(session);
    }

    /// Gives the agent's existing bounded retry ownership of pending redirects until this guard drops.
    #[must_use]
    pub fn agent_navigation(&self, session: &str) -> impl Drop + Send + use<> {
        let mut state = self.inner.lock();
        let entry = state.entry(session);
        entry.cancel_navigation();
        entry.agent_navigation += 1;
        AgentNavigation {
            inner: self.inner.clone(),
            session: session.to_owned(),
        }
    }
}

impl Hooks {
    pub(crate) fn begin_navigation(&self, label: &str) -> Option<u64> {
        let mut state = self.0.lock();
        let session = state.session_of(label)?.to_owned();
        let entry = state.entry(&session);
        entry.cancel_navigation();
        (entry.agent_navigation == 0)
            .then(|| entry.mount.as_ref().map(|m| m.navigation.generation))
            .flatten()
    }

    /// True means held: the engine cancels this attempt, with no refusal recorded yet.
    pub(crate) fn hold_navigation(&self, label: &str, url: &Url, generation: Option<u64>) -> bool {
        let Some(generation) = generation else {
            return false;
        };
        let Pre::Name {
            session,
            host,
            later: false,
            lan,
        } = self.pre(label, url)
        else {
            return false;
        };
        if self.0.names.quick(&host, crate::names::UI_WAIT).is_some() {
            return false;
        }
        let id = MountId {
            session_id: session.clone(),
            label: label.to_owned(),
        };
        {
            let mut state = self.0.lock();
            let Some(entry) = state.entries.get_mut(&session) else {
                return true;
            };
            let Some(mount) = entry
                .mount
                .as_mut()
                .filter(|m| m.label == label && m.navigation.generation == generation)
            else {
                return true;
            };
            mount.navigation.pending = Some(url.clone());
            entry.loading = true;
        }
        self.0.changed(&session);
        let inner = Arc::downgrade(&self.0);
        let url = url.clone();
        let name = host.clone();
        self.0.names.when_done(
            &host,
            Box::new(move |verdict| {
                let Some(inner) = inner.upgrade() else { return };
                let hooks = Hooks(inner.clone());
                if let Some(why) = verdict.refusal_on(lan) {
                    hooks.finish_navigation(
                        &id,
                        generation,
                        &url,
                        Some(&format!("{name} is not opened: {why}")),
                    );
                    return;
                }
                let Some(engine) = inner.engine.upgrade() else {
                    return;
                };
                let (queued_id, queued_url) = (id.clone(), url.clone());
                let result = engine.continue_navigation(
                    &id,
                    url.clone(),
                    Box::new(move || {
                        // This closure runs at native dispatch, not when the lookup completes. The normal
                        // hook checks again when the engine reports the new navigation, including DNS expiry.
                        let refusal = match hooks.decide(&queued_id.label, &queued_url) {
                            Ok(()) => None,
                            Err(Some((_, why))) => Some(why),
                            Err(None) => return false,
                        };
                        hooks.finish_navigation(
                            &queued_id,
                            generation,
                            &queued_url,
                            refusal.as_deref(),
                        )
                    }),
                );
                if let Err(error) = result {
                    Hooks(inner).finish_navigation(&id, generation, &url, Some(&error.to_string()));
                }
            }),
        );
        true
    }

    /// Consumes only the latest held attempt. Refusals and loading state change under the same lock.
    fn finish_navigation(
        &self,
        id: &MountId,
        generation: u64,
        url: &Url,
        reason: Option<&str>,
    ) -> bool {
        let mut state = self.0.lock();
        let Some(entry) = state.entries.get_mut(&id.session_id) else {
            return false;
        };
        let Some(mount) = entry.mount.as_mut().filter(|m| {
            m.label == id.label
                && m.navigation.generation == generation
                && m.navigation.pending.as_ref() == Some(url)
        }) else {
            return false;
        };
        mount.navigation.pending = None;
        if let Some(reason) = reason {
            entry.loading = false;
            entry.note_refused(url.as_str(), reason);
        } else {
            // A held redirect can replace an unfinished service open. Its own Started event must
            // become authoritative even when the old chain never emitted an admission/load event.
            entry.pending_open = Some(url.clone());
            entry.pending_requested_admitted = None;
            entry.pending_admitted_urls.clear();
            entry.pending_started = false;
            entry.title = None;
        }
        drop(state);
        if let Some(reason) = reason {
            self.0.guard_refused(&id.session_id, reason);
        }
        self.0.changed(&id.session_id);
        reason.is_none()
    }
}
