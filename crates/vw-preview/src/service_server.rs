//! Tracks whether a formerly opened local App preview still has a server owned by its session.
//! Depends on: preview state and URL policy. Called by: the core's port checks before list/reload and on its monitor.

use crate::policy::Destination;
use crate::service::PreviewService;
use crate::state::PreviewServerState;

impl PreviewService {
    /// Port of this page's current loopback URL, including when its mount was suspended.
    #[must_use]
    pub fn local_server_port(&self, page: &str) -> Option<u16> {
        let state = self.inner.lock();
        let entry = state.entries.get(page)?;
        if entry.lan_host.is_some() {
            return None;
        }
        let url = entry.url.as_ref()?;
        match self.inner.policy.classify(url).ok()? {
            Destination::Loopback { port } => Some(port),
            _ => None,
        }
    }

    /// Applies an attributed listener check for the exact URL port read before that check. A late
    /// scan cannot mark a newly opened URL stopped. Once stopped, a fresh checked open must restore
    /// its withdrawn port rights and page; a listener alone cannot make the old mount live again.
    /// Returns whether the public view changed.
    #[must_use]
    pub fn set_local_server_listening(&self, page: &str, port: u16, listening: bool) -> bool {
        let changed = {
            let mut state = self.inner.lock();
            let Some(entry) = state.entries.get_mut(page) else {
                return false;
            };
            let current = entry
                .url
                .as_ref()
                .filter(|_| entry.lan_host.is_none())
                .and_then(|url| self.inner.policy.classify(url).ok());
            if current != Some(Destination::Loopback { port }) {
                return false;
            }
            if listening || entry.server_status.is_some() {
                false
            } else {
                entry.server_status = Some(PreviewServerState::Stopped);
                true
            }
        };
        if changed {
            self.inner.changed(page);
        }
        changed
    }
}
