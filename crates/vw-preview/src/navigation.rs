//! Navigation lifecycle and cancellation for service operations.
//! Depends on hooks, registry and the engine continuation contract; called by hooks and service operations.
//! The browser engine handles DNS; no address-policy continuation reopens a URL.

use std::sync::Arc;

use url::Url;

use crate::hooks::{Hooks, Inner};
use crate::service::PreviewService;
use crate::state::Entry;

/// Distinct even for two attempts at the same URL. Mount labels also change on every remount.
#[derive(Clone, Debug, Default)]
pub(crate) struct Navigation {
    pub(crate) load_addresses: crate::load_addresses::LoadAddresses,
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
}
