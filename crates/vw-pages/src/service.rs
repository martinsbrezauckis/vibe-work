//! `PagesService`: Project Pages for the core. Page servers per pages folder, the one viewer webview (a second
//! preview-engine instance with the hooks of `viewer`), hubs, page facts, the section pick, comments and settings.
//!
//! Depends on: vw-preview (engine seam, layout, screenshots), `server`, `viewer`, `hub`, `page`, `pick`,
//! `comments`, `settings`, tokio. Called by: vw-core-api (`pages` commands, session end, quit).
//! Invariants:
//! - The state lock is never held across an engine call or a file scan.
//! - Every engine call is bounded (`engine_timeout`; a pick by `pick_timeout`, then cancelled).
//! - The viewer only ever loads URLs of a page server this service started (`viewer::allowed`).
//! - A pick's section is the first of the page's reported ids that exists in the page FILE; labels come from the
//!   file. A page script can make a pick cite another real section, never an invented one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;
use vw_preview::{
    DeviceWidth, EngineError, EngineFuture, HistoryStep, MountId, MountSpec, PreviewEngine, Rect,
    Shot, Storage, StorageKey,
};

use crate::comments::CommentStore;
use crate::config::{PagesConfig, PagesError, PickedSection};
use crate::paths::PagePath;
use crate::pick::Picked;
use crate::picking::{pick_on, section_of};
use crate::server::PagesServer;
use crate::settings::SettingsStore;
use crate::viewer::{Mount, Shared, State, Viewer, ViewerHooks, ViewerView};

/// Label prefix of the viewer webview (inside the `vw-preview-` namespace, so the app's navigation guard and
/// capability file already treat it as an untrusted child webview).
pub const LABEL_PREFIX: &str = "vw-preview-pages-";

/// Project Pages for every session.
#[derive(Clone)]
pub struct PagesService {
    pub(crate) shared: Arc<Shared>,
    pub(crate) engine: Arc<dyn PreviewEngine>,
    pub(crate) config: Arc<PagesConfig>,
    pub(crate) servers: Arc<AsyncMutex<HashMap<PathBuf, Arc<PagesServer>>>>,
    pub(crate) comments: Arc<CommentStore>,
    /// Answers to page inputs (Format 1.2 §6.6).
    pub(crate) answers: Arc<crate::answers::AnswerStore>,
    pub(crate) settings: Arc<SettingsStore>,
    /// The visual checker, when the app gave it an engine.
    pub(crate) checker: Option<Arc<crate::checker::VisualChecker>>,
    /// The app's theme the viewer passes to pages (`light` or `dark`; Format 1.2 §6.4).
    pub(crate) theme: Arc<std::sync::Mutex<String>>,
    /// Built hubs by handbook folder, with the folder fingerprint they were built from (`files::fingerprint`).
    pub(crate) hubs: Arc<std::sync::Mutex<HashMap<PathBuf, (u64, crate::hub::Hub)>>>,
}

pub(crate) async fn bounded<T>(
    limit: Duration,
    what: &str,
    future: EngineFuture<'_, T>,
) -> Result<T, EngineError> {
    tokio::time::timeout(limit, future)
        .await
        .map_err(|_| EngineError::Timeout(what.to_owned()))?
}

impl PagesService {
    /// A service over `engine` (its hooks are attached here).
    #[must_use]
    pub fn new(engine: Arc<dyn PreviewEngine>, config: PagesConfig) -> Self {
        let shared = Arc::new(Shared {
            state: std::sync::Mutex::new(State::default()),
            changes: tokio::sync::broadcast::channel(64).0,
            loads: tokio::sync::watch::channel(0).0,
            pdf: config.pdf,
        });
        engine.attach(Arc::new(ViewerHooks(shared.clone())));
        Self {
            comments: Arc::new(CommentStore::new(config.data_dir.join("comments"))),
            answers: Arc::new(crate::answers::AnswerStore::new(
                config.data_dir.join("answers"),
            )),
            settings: Arc::new(SettingsStore::new(config.data_dir.join("projects"))),
            shared,
            engine,
            config: Arc::new(config),
            servers: Arc::new(AsyncMutex::new(HashMap::new())),
            hubs: Arc::new(std::sync::Mutex::new(HashMap::new())),
            checker: None,
            theme: Arc::new(std::sync::Mutex::new("dark".to_owned())),
        }
    }

    /// The same service with a visual checker on its own engine; `storage` is the checker webview's data
    /// folder.
    #[must_use]
    pub fn with_checker(
        mut self,
        engine: Arc<dyn PreviewEngine>,
        storage: std::path::PathBuf,
    ) -> Self {
        self.checker = Some(Arc::new(crate::checker::VisualChecker::new(
            engine, storage,
        )));
        self
    }

    /// Sessions whose viewer changed (`None`: all).
    #[must_use]
    pub fn changes(&self) -> tokio::sync::broadcast::Receiver<Option<String>> {
        self.shared.changes.subscribe()
    }

    /// The comment store.
    #[must_use]
    pub fn comments(&self) -> &CommentStore {
        &self.comments
    }

    /// A session's viewer.
    #[must_use]
    pub fn view(&self, session: &str) -> Option<ViewerView> {
        self.shared.lock().view(session)
    }

    /// Where the window shows the viewer: a session and the tab's page area, or nothing (hidden).
    ///
    /// # Errors
    /// [`PagesError::Other`] for a bad rectangle; engine failures.
    pub async fn layout(&self, shown: Option<(String, Rect)>) -> Result<(), PagesError> {
        if let Some((_, rect)) = &shown {
            rect.checked().map_err(PagesError::Other)?;
        }
        self.shared.lock().shown.clone_from(&shown);
        match shown {
            Some((session, rect)) if self.shared.lock().viewers.contains_key(&session) => {
                self.show(&session, rect).await
            }
            _ => self.hide().await,
        }
    }

    async fn hide(&self) -> Result<(), PagesError> {
        let mount = self.shared.lock().mount.clone();
        if let Some(mount) = mount {
            let id = mount_id(&mount);
            bounded(
                self.config.engine_timeout,
                "hide",
                self.engine.place(&id, None),
            )
            .await?;
        }
        Ok(())
    }

    /// Places the viewer over `rect` showing `session`'s page (mounting or re-targeting it).
    pub(crate) async fn show(&self, session: &str, rect: Rect) -> Result<(), PagesError> {
        let placement = vw_preview::place(rect, DeviceWidth::Fit).map_err(PagesError::Other)?;
        let (mount, viewer) = {
            let state = self.shared.lock();
            (state.mount.clone(), state.viewers.get(session).cloned())
        };
        let Some(viewer) = viewer else {
            return self.hide().await;
        };
        let url = viewer
            .url()
            .ok_or_else(|| PagesError::Other("the page URL is not valid".to_owned()))?;
        let timeout = self.config.engine_timeout;
        match mount {
            Some(mount) => {
                let id = mount_id(&mount);
                let navigate = mount.session != session
                    || mount.port != viewer.port
                    || mount.path != viewer.path
                    || mount.anchor != viewer.anchor;
                {
                    let mut state = self.shared.lock();
                    if let Some(m) = state.mount.as_mut() {
                        session.clone_into(&mut m.session);
                        m.port = viewer.port;
                        m.path.clone_from(&viewer.path);
                        m.anchor.clone_from(&viewer.anchor);
                    }
                    if navigate {
                        loading(&mut state, session);
                    }
                }
                if navigate {
                    tracing::debug!(label = %id.label, path = ?viewer.path, "page viewer navigates");
                    bounded(timeout, "navigate", self.engine.navigate(&id, url)).await?;
                }
                bounded(timeout, "place", self.engine.place(&id, Some(placement))).await?;
            }
            None => self.mount(session, &viewer, url, placement).await?,
        }
        Ok(())
    }

    async fn mount(
        &self,
        session: &str,
        viewer: &Viewer,
        url: url::Url,
        placement: vw_preview::Placement,
    ) -> Result<(), PagesError> {
        let label = {
            let mut state = self.shared.lock();
            loading(&mut state, session);
            state.generation += 1;
            let label = format!("{}{}", self.config.label_prefix, state.generation);
            state.mount = Some(Mount {
                label: label.clone(),
                session: session.to_owned(),
                port: viewer.port,
                path: viewer.path.clone(),
                anchor: viewer.anchor.clone(),
            });
            label
        };
        let spec = MountSpec {
            id: MountId {
                session_id: session.to_owned(),
                label: label.clone(),
            },
            url,
            storage: Storage {
                key: StorageKey::of("vibe-work-pages-viewer"),
                dir: self.config.view_storage.clone(),
            },
            // Format 1.2 §6.4: every page this webview loads gets the app's theme on `<html data-theme>` before it
            // paints (a document-start script, exempt from the page's CSP; it runs no page code).
            console_script: crate::theme::theme_script(&self.theme()),
            console_drain: "'[]'".to_owned(),
            context_menu: false,
            placement: Some(placement),
            parked: None,
            rules: None,
        };
        if let Err(error) =
            bounded(self.config.engine_timeout, "mount", self.engine.mount(spec)).await
        {
            self.shared.lock().mount = None;
            return Err(error.into());
        }
        Ok(())
    }

    /// The mount when it shows `session`.
    fn mount_of(&self, session: &str) -> Result<MountId, PagesError> {
        let state = self.shared.lock();
        let shown = state.shown.as_ref().is_some_and(|(s, _)| s == session);
        match state.mount.as_ref() {
            Some(mount) if shown && mount.session == session => Ok(mount_id(mount)),
            _ => Err(PagesError::NotShown),
        }
    }

    /// Back, forward or reload in the viewer of `session` (shown).
    ///
    /// # Errors
    /// [`PagesError::NotShown`], engine failures.
    pub async fn history(&self, session: &str, step: HistoryStep) -> Result<(), PagesError> {
        let id = self.mount_of(session)?;
        bounded(
            self.config.engine_timeout,
            "history",
            self.engine.history(&id, step),
        )
        .await?;
        Ok(())
    }

    /// Forgets the last refused external link of `session`.
    pub fn clear_external(&self, session: &str) {
        if let Some(viewer) = self.shared.lock().viewers.get_mut(session) {
            viewer.external = None;
        }
        self.shared.changed(Some(session));
    }

    /// Lets the user point at a part of `session`'s shown page (selection, else the next click) and returns what
    /// was picked (untrusted page data).
    ///
    /// # Errors
    /// [`PagesError::NotShown`], [`PagesError::NothingPicked`], engine failures.
    pub async fn pick_element(&self, session: &str) -> Result<Picked, PagesError> {
        let id = self.mount_of(session)?;
        pick_on(&*self.engine, &id, &self.config, &self.shared).await
    }

    /// Picks a SECTION of the page shown for `session`: the first reported id that exists in the page file.
    ///
    /// # Errors
    /// As [`Self::pick_element`], plus [`PagesError::NoSection`] and [`PagesError::NoPage`] (the hub is shown).
    pub async fn pick_section(
        &self,
        session: &str,
        project: &Path,
    ) -> Result<PickedSection, PagesError> {
        let picked = self.pick_element(session).await?;
        let path = self
            .view(session)
            .and_then(|v| v.path)
            .ok_or_else(|| PagesError::NoPage("(the hub)".to_owned()))?;
        let page = PagePath::parse(&path)?;
        let (service, project_owned, page_owned) =
            (self.clone(), project.to_path_buf(), page.clone());
        let facts = tokio::task::spawn_blocking(move || service.page(&project_owned, &page_owned))
            .await
            .map_err(|e| PagesError::Other(e.to_string()))??;
        section_of(&facts, &page, &picked).ok_or(PagesError::NoSection)
    }

    /// Cancels a running pick (the overlay goes away).
    pub async fn cancel_pick(&self) {
        let (name, mount) = {
            let mut state = self.shared.lock();
            (state.picking.take(), state.mount.clone())
        };
        if let (Some(name), Some(mount)) = (name, mount) {
            let id = mount_id(&mount);
            let _ = bounded(
                self.config.engine_timeout,
                "cancel pick",
                self.engine.evaluate(&id, crate::pick::cancel_script(&name)),
            )
            .await;
        }
    }

    /// A shaped screenshot of the viewer showing `session`.
    ///
    /// # Errors
    /// [`PagesError::NotShown`], engine and image failures.
    pub async fn screenshot(&self, session: &str) -> Result<Shot, PagesError> {
        let id = self.mount_of(session)?;
        let png = bounded(
            self.config.engine_timeout,
            "screenshot",
            self.engine.screenshot(&id),
        )
        .await?;
        vw_preview::shape(&png).map_err(PagesError::Other)
    }

    /// Reloads the shown page when its file changed since it loaded. Returns true when it reloaded. Never while a
    /// navigation the service sent is still loading: on `WebView2` the first load of a new profile takes hundreds of
    /// milliseconds, and a reload in that window reloads the previous document (`about:blank`) and cancels the
    /// navigation (the same can happen on `WebKitGTK` with a PDF,
    /// which reports no `Started`).
    pub async fn reload_if_changed(&self) -> bool {
        let target = {
            let state = self.shared.lock();
            let Some(mount) = state.mount.clone() else {
                return false;
            };
            let Some(viewer) = state.viewers.get(&mount.session) else {
                return false;
            };
            if viewer.loading
                || state
                    .shown
                    .as_ref()
                    .is_none_or(|(s, _)| *s != mount.session)
            {
                return false;
            }
            viewer.file().map(|f| (mount, f, viewer.loaded_mtime))
        };
        let Some((mount, file, loaded)) = target else {
            return false;
        };
        let now = std::fs::metadata(&file)
            .ok()
            .and_then(|m| m.modified().ok());
        // `loaded` is `None` until the page reported loaded: nothing to compare yet (a reload now would only reload
        // the document the navigation is replacing).
        if now.is_none() || loaded.is_none() || now == loaded {
            return false;
        }
        if let Some(viewer) = self.shared.lock().viewers.get_mut(&mount.session) {
            viewer.loaded_mtime = now;
        }
        let id = mount_id(&mount);
        tracing::debug!(label = %id.label, "page viewer reloads a changed file");
        let reloaded = bounded(
            self.config.engine_timeout,
            "reload",
            self.engine.history(&id, HistoryStep::Reload),
        )
        .await
        .is_ok();
        self.shared.changed(Some(&mount.session));
        reloaded
    }

    /// A session ended: its viewer is forgotten (and closed when it shows that session).
    pub async fn session_ended(&self, session: &str) {
        let mount = {
            let mut state = self.shared.lock();
            state.viewers.remove(session);
            if state.mount.as_ref().is_some_and(|m| m.session == session) {
                state.mount.take()
            } else {
                None
            }
        };
        if let Some(mount) = mount {
            let _ = bounded(
                self.config.engine_timeout,
                "close",
                self.engine.close(&mount_id(&mount)),
            )
            .await;
        }
    }

    /// Closes the viewer and stops every page server (quit).
    pub async fn shutdown(&self) {
        let mount = self.shared.lock().mount.take();
        if let Some(mount) = mount {
            let _ = bounded(
                self.config.engine_timeout,
                "close",
                self.engine.close(&mount_id(&mount)),
            )
            .await;
        }
        self.servers.lock().await.clear();
    }
}

pub(crate) fn mount_id(mount: &Mount) -> MountId {
    MountId {
        session_id: mount.session.clone(),
        label: mount.label.clone(),
    }
}

/// Marks `session`'s viewer as loading a navigation the service is about to send (before the engine reports it), so
/// the reload tick leaves it alone until the page reports loaded.
fn loading(state: &mut crate::viewer::State, session: &str) {
    if let Some(viewer) = state.viewers.get_mut(session) {
        viewer.loading = true;
        viewer.loaded_mtime = None;
    }
}
