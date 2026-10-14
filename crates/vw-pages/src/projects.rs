//! `PagesService`'s project data (split out of `service`): per-project settings, the handbook folder (chosen, else
//! `docs/handbook/`, else a pre-1.0 `docs/pages/`), its page server (started on first use), one page's facts and the
//! current plan. No engine calls here.
//!
//! Depends on: `service` (the struct), `server`, `settings`, `hub`, `handbook`, `page`, `paths`. Called by:
//! vw-core-api, `service`.
//! Invariant: a page is read only when its file lies inside the project's handbook folder (`files::inside`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::PagesError;
use crate::page::PageFacts;
use crate::paths::PagePath;
use crate::server::PagesServer;
use crate::service::PagesService;
use crate::settings::PagesSettings;

impl PagesService {
    /// The settings of a project.
    #[must_use]
    pub fn settings(&self, project: &Path) -> PagesSettings {
        self.settings.get(project).0
    }

    /// The handbook folder of a project (relative) and whether it is the pre-1.0 one.
    #[must_use]
    pub fn folder(&self, project: &Path) -> crate::handbook::Folder {
        let (settings, legacy) = self.settings.get(project);
        crate::handbook::Folder {
            relative: settings.folder,
            legacy,
        }
    }

    /// Saves a project's settings; a running server follows the scripts switch, a moved folder gets a new server.
    ///
    /// # Errors
    /// [`PagesError::Other`] with the reason.
    pub async fn set_settings(
        &self,
        project: &Path,
        settings: &PagesSettings,
    ) -> Result<PagesSettings, PagesError> {
        let old = self.settings(project);
        self.settings
            .set(project, settings)
            .map_err(PagesError::Other)?;
        let saved = self.settings(project);
        let mut servers = self.servers.lock().await;
        if old.folder != saved.folder {
            servers.remove(&old.pages_dir(project));
        } else if let Some(server) = servers.get(&saved.pages_dir(project)) {
            server.set_scripts(saved.scripts);
        }
        drop(servers);
        self.shared.changed(None);
        Ok(saved)
    }

    /// The pages folder of a project.
    #[must_use]
    pub fn pages_dir(&self, project: &Path) -> PathBuf {
        self.settings(project).pages_dir(project)
    }

    /// The page server of a project, started when needed.
    ///
    /// # Errors
    /// [`PagesError::NoPagesFolder`]; a bind failure.
    pub async fn server(&self, project: &Path) -> Result<Arc<PagesServer>, PagesError> {
        let settings = self.settings(project);
        let dir = settings.pages_dir(project);
        if !dir.is_dir() {
            return Err(PagesError::NoPagesFolder(settings.folder));
        }
        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get(&dir) {
            return Ok(server.clone());
        }
        let name = project.file_name().map_or_else(
            || "Project".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        let server = Arc::new(
            PagesServer::start(dir.clone(), settings.scripts, name)
                .await
                .map_err(|e| PagesError::Other(format!("the page server could not start: {e}")))?,
        );
        servers.insert(dir, server.clone());
        Ok(server)
    }

    /// The facts of one page of a project (blocking file read: call from a blocking task).
    ///
    /// # Errors
    /// [`PagesError::NoPage`].
    pub fn page(&self, project: &Path, path: &PagePath) -> Result<PageFacts, PagesError> {
        let file = path.under(&self.pages_dir(project));
        let root = self.pages_dir(project);
        if !file.is_file() || !crate::handbook::files::inside(&root, &file) {
            return Err(PagesError::NoPage(path.to_string()));
        }
        crate::handbook::files::read_page(&file, path.as_str())
            .map(|scan| scan.facts)
            .ok_or_else(|| PagesError::NoPage(path.to_string()))
    }

    /// The current plan of a project (blocking): the newest `active` plan, else the newest open plan, else the
    /// newest plan of the archive.
    #[must_use]
    pub fn current_plan(&self, project: &Path) -> Option<PagePath> {
        let folder = self.folder(project);
        let hub = crate::hub::build(&self.pages_dir(project), &folder);
        let plans = |group: &str| -> Vec<&crate::hub::HubPage> {
            hub.groups
                .iter()
                .filter(|g| g.id == group)
                .flat_map(|g| g.pages.iter())
                .filter(|p| p.kind.as_deref() == Some("plan"))
                .collect()
        };
        let open = plans("plans");
        open.iter()
            .find(|p| p.status.as_deref() == Some("active"))
            .or_else(|| open.first())
            .copied()
            .or_else(|| plans("archive").first().copied())
            .and_then(|p| PagePath::parse(&p.path).ok())
    }

    /// The live hub of a project's handbook (blocking); `None` when the folder does not exist. Rebuilt only when
    /// the folder's fingerprint changed since the last build (the same answer otherwise, from the cache).
    #[must_use]
    pub fn hub(&self, project: &Path) -> Option<crate::hub::Hub> {
        let root = self.pages_dir(project);
        if !root.is_dir() {
            return None;
        }
        let folder = self.folder(project);
        let print = crate::handbook::files::fingerprint(&root);
        let cached = self
            .hubs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&root)
            .filter(|(p, hub)| *p == print && hub.folder == folder.relative)
            .map(|(_, hub)| hub.clone());
        if cached.is_some() {
            return cached;
        }
        let hub = crate::hub::build(&root, &folder);
        self.hubs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(root, (print, hub.clone()));
        Some(hub)
    }

    /// The Projects overview card of a project (blocking; heads only, Handbook Format 1.2 §3).
    #[must_use]
    pub fn card(&self, project: &Path) -> crate::handbook::overview::ProjectCard {
        let folder = self.folder(project);
        crate::handbook::overview::card(project, &folder.relative, folder.legacy)
    }
}
