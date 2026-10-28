//! The visual check in the core ("Visual check of agent-written pages"): background checks of the pages
//! a session's Project tab shows (so a badge is ready before the user opens a page, and the agent's next
//! `handbook_visual_check` answers from the cache), and the cached reports the hub answer carries.
//!
//! Depends on: vw-pages (`PagesService::{visual_check, visual_reports, hub}`), `pages` (the hub and its task
//! slot), `core::Shared`. Called by: `pages` (open), `pages_overview` (hub), `agent_tools::handbook_tool`.
//! Invariants:
//! - At most one background task at a time, owned by the `PagesHub` (aborted on quit); a request while it runs is
//!   dropped (the next hub load asks again).
//! - At most [`MAX_QUEUED`] pages per run, the newest first; a page whose page and stylesheet bytes did not change
//!   is answered from the cache (no render).
//! - `pagesChanged` (for every session) only after a check actually ran.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use crate::core::Shared;
use crate::event::CoreEvent;

/// Most pages checked per background run.
pub(crate) const MAX_QUEUED: usize = 20;

/// Checks `pages` of `project` in the background (first given first), unless a run is already going.
pub(crate) fn queue_checks(shared: &Arc<Shared>, project: &Path, pages: Vec<String>) {
    let Some(hub) = &shared.pages else { return };
    if !hub.service.can_check() || pages.is_empty() {
        return;
    }
    let mut slot = hub.checks.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.as_ref().is_some_and(|task| !task.is_finished()) {
        return;
    }
    let (service, project): (_, PathBuf) = (hub.service.clone(), project.to_path_buf());
    let weak = Arc::downgrade(shared);
    *slot = Some(tokio::spawn(async move {
        let mut ran = false;
        for page in pages.into_iter().take(MAX_QUEUED) {
            let Ok(path) = vw_pages::PagePath::parse(&page) else {
                continue;
            };
            let cached = {
                let (s, r, p) = (service.clone(), project.clone(), path.clone());
                tokio::task::spawn_blocking(move || s.visual_reports(&r, &[p.to_string()]))
                    .await
                    .map_or(true, |m| !m.is_empty())
            };
            if cached {
                continue;
            }
            match service.visual_check(&project, &path).await {
                Ok(_) => ran = true,
                Err(error) => {
                    tracing::debug!(%error, page, "background visual check skipped");
                }
            }
        }
        if ran && let Some(shared) = weak.upgrade() {
            shared.emit(CoreEvent::PagesChanged { session_id: None });
        }
    }));
}

/// The HTML pages of a hub worth checking, newest first (Markdown pages are shown by the app's own
/// frame; the live index is the app's).
pub(crate) fn pages_to_check(hub: &vw_pages::Hub) -> Vec<String> {
    let mut pages: Vec<&vw_pages::HubPage> = hub.pages().filter(|p| !p.lite).collect();
    pages.sort_by_key(|p| std::cmp::Reverse(p.modified_ms));
    pages.into_iter().map(|p| p.path.clone()).collect()
}
