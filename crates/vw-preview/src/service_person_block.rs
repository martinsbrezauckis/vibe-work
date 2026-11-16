//! The core's route to the built-in browser's input block: sets or lifts the block of the page views the agent
//! controls in one chat (the web pages it opened and a page the person handed to it). The person's own pages and the
//! App preview are never blocked. The rule of when the block holds lives in the core; the views read it through
//! [`crate::person_block`].
//! Depends on the preview registry (pages, their owners and live mounts). Called by the core's browser input block
//! sync, the mount and the page hand-over.

use std::time::Instant;

use super::PreviewService;
use crate::state::State;
use crate::tabs::PageTabOwner;

/// Whether the agent's input block reaches `page`: the web pages the agent controls (its own, one handed to it).
/// The person's own pages, the App preview (the agent's browser input never acts there) and a page whose role is
/// unknown are never blocked.
fn in_agent_scope(state: &State, page: &str) -> bool {
    let owner = state.owner_of(page);
    state
        .tabs
        .get(&owner)
        .and_then(|tabs| tabs.owners.get(page))
        .is_some_and(|kind| matches!(kind, PageTabOwner::Agent | PageTabOwner::DelegatedToAgent))
}

/// Points the view `label` of `page` at its chat's block lease when the agent controls the page; otherwise the view
/// reads no lease. The one place that decides which views a chat's block reaches.
pub(crate) fn scope_view(state: &State, page: &str, label: &str) {
    if in_agent_scope(state, page) {
        crate::person_block::assign(label, &state.owner_of(page));
    } else {
        crate::person_block::forget(label);
    }
}

impl PreviewService {
    /// Blocks the person's input in the page views the agent controls in `owner` until `until` (`None` lifts it at
    /// once). Views mounted later read the same lease.
    pub fn set_agent_input_block(&self, owner: &str, until: Option<Instant>) {
        {
            let state = self.inner.lock();
            let pages = state
                .tabs
                .get(owner)
                .map(|tabs| tabs.web.clone())
                .unwrap_or_default();
            for page in std::iter::once(owner.to_owned()).chain(pages) {
                if let Some(mount) = state
                    .entries
                    .get(&page)
                    .and_then(|entry| entry.mount.as_ref())
                {
                    scope_view(&state, &page, &mount.label);
                }
            }
        }
        crate::person_block::set(owner, until);
    }
}
