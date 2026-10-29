//! Opening a page in a session's viewer: which page (or the hub), scrolled to which section or step,
//! loaded at once when the session is shown. Split out of `service`.
//!
//! Depends on: `service`, `viewer`, `paths`, `handbook::format` (the id pattern). Called by: vw-core-api (`open`,
//! `plan`), the Projects overview's opener.

use std::path::Path;

use crate::config::PagesError;
use crate::paths::PagePath;
use crate::service::PagesService;
use crate::viewer::{Viewer, ViewerView};

impl PagesService {
    /// Shows `path` (or the hub, `None`) of `project` in `session`'s viewer; loads it at once when the session is
    /// shown, and waits (bounded) for the load.
    ///
    /// # Errors
    /// [`PagesError`] (no folder, no page, engine failure).
    pub async fn open(
        &self,
        session: &str,
        project: &Path,
        path: Option<PagePath>,
    ) -> Result<ViewerView, PagesError> {
        self.open_at(session, project, path, None).await
    }

    /// [`Self::open`] scrolled to section or step `anchor` of the page (the plan checklist).
    ///
    /// # Errors
    /// As [`Self::open`]; [`PagesError::Other`] for an anchor outside the section-id pattern.
    pub async fn open_at(
        &self,
        session: &str,
        project: &Path,
        path: Option<PagePath>,
        anchor: Option<String>,
    ) -> Result<ViewerView, PagesError> {
        if let Some(anchor) = &anchor
            && (anchor.len() > 100 || !crate::handbook::format::is_id(anchor))
        {
            return Err(PagesError::Other(format!(
                "\"{}\" is not a section id (a-z, 0-9 and -)",
                crate::html::clip(anchor, 40)
            )));
        }
        let server = self.server(project).await?;
        if let Some(path) = &path
            && !path.under(server.root()).is_file()
        {
            return Err(PagesError::NoPage(path.to_string()));
        }
        let before = *self.shared.loads.borrow();
        let mut loads = self.shared.loads.subscribe();
        {
            let mut state = self.shared.lock();
            let viewer = state.viewers.entry(session.to_owned()).or_default();
            let previous = if viewer.project == project && viewer.path == path {
                viewer.clone()
            } else {
                Viewer::default()
            };
            let mut next = Viewer {
                project: project.to_path_buf(),
                pages_dir: server.root().to_path_buf(),
                port: server.port(),
                base_url: server.base_url(),
                path,
                anchor,
                ..previous
            };
            // Loading from the reset on, by `show()`'s own navigate rule (as `open_file`): the page stays loading
            // until it reports loaded, whether `show()` runs below (viewer shown) or at the next layout (hidden now).
            // A page the mount already shows is not navigated again and keeps its own state.
            next.loading |= crate::service::navigates(state.mount.as_ref(), session, &next);
            state.viewers.insert(session.to_owned(), next);
        }
        if self.show_current(Some(session)).await? {
            let _ =
                tokio::time::timeout(self.config.load_wait, loads.wait_for(|n| *n > before)).await;
        }
        self.shared.changed(Some(session));
        self.view(session).ok_or(PagesError::NotShown)
    }
}
