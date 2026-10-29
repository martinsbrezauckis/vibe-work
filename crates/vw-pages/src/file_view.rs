//! The file preview's use of the viewer: show one HTML page or PDF from ANY folder the core allowed (the
//! session folder or a folder the user picked), in a second `PagesService` instance whose webview has its own label
//! prefix ([`crate::FILES_LABEL_PREFIX`]). Same isolation as the Project tab: the page server's CSP, navigation only
//! to that server's port, no capability for the label.
//!
//! Depends on: `service` (viewer state, show/load), `server`, `paths`. Called by: vw-core-api (`files view`).
//! Invariants:
//! - The server root is the folder of the file (the caller passes it); the path is a checked [`PagePath`] relative
//!   to it, so the served tree is never wider than that folder (no `..`, no hidden segments, canonical-root rule of
//!   the server).
//! - The caller (vw-core-api `files`) has already checked that the file lies inside an allowed folder.

use std::path::Path;
use std::sync::Arc;

use crate::config::PagesError;
use crate::paths::PagePath;
use crate::server::PagesServer;
use crate::service::PagesService;
use crate::viewer::{Viewer, ViewerView};

/// Most folder servers the file preview keeps (each preview of a file in another folder starts one); the oldest
/// is stopped when a new one is needed.
const MAX_FOLDER_SERVERS: usize = 8;

impl PagesService {
    /// The page server for `root` (any folder), started on first use; scripts on, like the Project tab's default.
    async fn folder_server(&self, root: &Path) -> Result<Arc<PagesServer>, PagesError> {
        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get(root) {
            return Ok(server.clone());
        }
        if servers.len() >= MAX_FOLDER_SERVERS {
            // Stop a server no viewer shows (the map has no order; any idle one will do).
            let busy: Vec<u16> = self
                .shared
                .lock()
                .viewers
                .values()
                .map(|v| v.port)
                .collect();
            let idle = servers
                .iter()
                .find(|(_, s)| !busy.contains(&s.port()))
                .map(|(k, _)| k.clone());
            if let Some(key) = idle {
                servers.remove(&key);
            }
        }
        let name = root
            .file_name()
            .map_or_else(|| "Folder".to_owned(), |n| n.to_string_lossy().into_owned());
        let server = Arc::new(
            PagesServer::start_for_files(root.to_path_buf(), name)
                .await
                .map_err(|e| PagesError::Other(format!("the page server could not start: {e}")))?,
        );
        servers.insert(root.to_path_buf(), server.clone());
        Ok(server)
    }

    /// Shows `path` (an HTML page or a PDF, relative to `root`) in `session`'s viewer of this service; loads it at
    /// once when the session is shown and waits (bounded) for the load. The view reads `loading` from this call until
    /// the file's page reports loaded (also while the viewer is hidden and the load waits for the next layout).
    ///
    /// # Errors
    /// [`PagesError::NoPage`] when the file is not there; server and engine failures.
    pub async fn open_file(
        &self,
        session: &str,
        root: &Path,
        path: PagePath,
    ) -> Result<ViewerView, PagesError> {
        if !path.under(root).is_file() {
            return Err(PagesError::NoPage(path.to_string()));
        }
        let server = self.folder_server(root).await?;
        let before = *self.shared.loads.borrow();
        let mut loads = self.shared.loads.subscribe();
        {
            let mut state = self.shared.lock();
            let mut viewer = Viewer {
                pages_dir: server.root().to_path_buf(),
                port: server.port(),
                base_url: server.base_url(),
                path: Some(path),
                ..Viewer::default()
            };
            // Loading from the reset on, by `show()`'s own navigate rule: the file stays loading until its page
            // reports loaded, whether `show()` runs below (viewer shown) or at the window's next layout (hidden now).
            // A file the mount already shows is not navigated again, so it is not marked loading.
            viewer.loading = crate::service::navigates(state.mount.as_ref(), session, &viewer);
            state.viewers.insert(session.to_owned(), viewer);
        }
        if self.show_current(Some(session)).await? {
            let _ =
                tokio::time::timeout(self.config.load_wait, loads.wait_for(|n| *n > before)).await;
        }
        self.shared.changed(Some(session));
        self.view(session).ok_or(PagesError::NotShown)
    }
}
