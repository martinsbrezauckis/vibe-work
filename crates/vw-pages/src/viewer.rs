//! The page viewer's state and the engine hooks: which session's page the ONE viewer webview shows, on which page
//! server port, and the navigation rule for agent pages (`PROJECT_PAGES` "Security": no top navigation away from
//! the project's own page server).
//!
//! Depends on: vw-preview (engine hooks, `Rect`), url, `paths`. Called by: `service`, the engine (through
//! [`ViewerHooks`]).
//! Invariants:
//! - A navigation of the viewer is allowed only to `about:blank` or `http://127.0.0.1|localhost:<its page server
//!   port>/...`; everything else is refused. A refused `http`, `https` or `mailto` link is kept as the viewer's
//!   `external` link so the Pages tab can offer it (never opened by itself).
//! - New windows and downloads are always refused.
//! - Hooks hold the state lock only briefly and never call the engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use url::{Host, Url};
use vw_preview::{PageEvent, PreviewHooks, Rect};

use crate::paths::PagePath;

/// A session's viewer as the UI sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ViewerView {
    /// The session.
    pub session_id: String,
    /// The page shown (`None`: the hub at the server's root).
    pub path: Option<String>,
    /// The page's title (untrusted page text, at most 200 characters).
    pub title: Option<String>,
    /// A load is running.
    pub loading: bool,
    /// The last link the page tried to open outside the page server (offered, never opened by itself).
    pub external: Option<String>,
    /// The viewer webview shows this session now.
    pub shown: bool,
    /// The page server's origin (`http://127.0.0.1:<port>`).
    pub origin: Option<String>,
}

/// One session's viewer.
#[derive(Clone, Debug, Default)]
pub(crate) struct Viewer {
    /// The owning project; inline writes use the same answer path as the app panel.
    pub(crate) project: PathBuf,
    /// The pages folder it shows.
    pub(crate) pages_dir: PathBuf,
    /// The page server port.
    pub(crate) port: u16,
    /// Content base URL with the server's capability; never sent to the UI as the origin.
    pub(crate) base_url: String,
    pub(crate) path: Option<PagePath>,
    /// A section or step id to scroll to (`#anchor`; a click in the plan checklist), checked by the caller
    /// against the section-id pattern.
    pub(crate) anchor: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) loading: bool,
    pub(crate) external: Option<String>,
    /// Modification time of the page file when it was last loaded (reload when it changes).
    pub(crate) loaded_mtime: Option<std::time::SystemTime>,
}

impl Viewer {
    /// The URL of the viewer's page.
    pub(crate) fn url(&self) -> Option<Url> {
        let path = self.path.as_ref().map_or_else(String::new, |p| {
            p.as_str()
                .split('/')
                .map(encode_segment)
                .collect::<Vec<_>>()
                .join("/")
        });
        let fragment = self
            .anchor
            .as_ref()
            .map_or_else(String::new, |a| format!("#{a}"));
        Url::parse(&format!("{}{path}{fragment}", self.base_url)).ok()
    }

    /// The page file, when a page (not the hub) is shown.
    pub(crate) fn file(&self) -> Option<PathBuf> {
        self.path.as_ref().map(|p| p.under(&self.pages_dir))
    }
}

/// Percent-encodes one path segment.
fn encode_segment(segment: &str) -> String {
    segment
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The mounted viewer webview.
#[derive(Clone, Debug)]
pub(crate) struct Mount {
    pub(crate) label: String,
    /// Project of the loaded document, retained while a new viewer target is prepared.
    pub(crate) project: PathBuf,
    /// The session whose page it shows.
    pub(crate) session: String,
    pub(crate) port: u16,
    /// The page it loaded last (told by the service or seen in a page event), to avoid reloading on layout.
    pub(crate) path: Option<PagePath>,
    /// The anchor of the last navigation the service sent.
    pub(crate) anchor: Option<String>,
}

/// Everything the viewer knows.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) viewers: HashMap<String, Viewer>,
    pub(crate) mount: Option<Mount>,
    pub(crate) shown: Option<(String, Rect)>,
    pub(crate) generation: u64,
    /// Accessor of the running pick.
    pub(crate) picking: Option<String>,
}

impl State {
    pub(crate) fn view(&self, session: &str) -> Option<ViewerView> {
        let viewer = self.viewers.get(session)?;
        let shown = self.mount.as_ref().is_some_and(|m| m.session == session)
            && self.shown.as_ref().is_some_and(|(s, _)| s == session);
        Some(ViewerView {
            session_id: session.to_owned(),
            path: viewer.path.as_ref().map(ToString::to_string),
            title: viewer.title.clone(),
            loading: viewer.loading,
            external: viewer.external.clone(),
            shown,
            origin: Some(format!("http://127.0.0.1:{}", viewer.port)),
        })
    }
}

/// Shared state + change channel.
pub(crate) struct Shared {
    pub(crate) state: Mutex<State>,
    /// Session ids whose viewer changed (`None`: re-read every viewer).
    pub(crate) changes: tokio::sync::broadcast::Sender<Option<String>>,
    /// Bumped on every finished load (waiters for `open`).
    pub(crate) loads: tokio::sync::watch::Sender<u64>,
    /// Right clicks on the shown page (`page_menu`).
    pub(crate) menus: tokio::sync::broadcast::Sender<crate::page_menu::PageMenuAt>,
    /// A loaded PDF counts as the shown file (the File tab's viewer).
    pub(crate) pdf: bool,
}

impl Shared {
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn changed(&self, session: Option<&str>) {
        let _ = self.changes.send(session.map(str::to_owned));
    }
}

/// True when `url` is the page server on `port` (or `about:blank`).
#[must_use]
pub fn allowed(url: &Url, port: u16) -> bool {
    if url.as_str() == "about:blank" {
        return true;
    }
    let loopback = match url.host() {
        Some(Host::Ipv4(v4)) => v4.octets() == [127, 0, 0, 1],
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        _ => false,
    };
    url.scheme() == "http"
        && loopback
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(port)
}

/// The page path of a URL on the page server (`None`: the root, i.e. the hub, or not a page).
fn path_of(url: &Url, pdf: bool) -> Option<PagePath> {
    // The router checks the first segment; viewer state tracks only the document's relative path.
    let (_, path) = url.path().strip_prefix('/')?.split_once('/')?;
    let decoded = crate::links::percent_decode(path);
    if pdf {
        PagePath::viewable(&decoded).ok()
    } else {
        PagePath::parse(&decoded).ok()
    }
}

/// The hooks handed to the viewer's engine.
pub(crate) struct ViewerHooks(pub(crate) Arc<Shared>);

impl ViewerHooks {
    fn note_external(&self, label: &str, url: &Url) {
        let mut state = self.0.lock();
        let Some(session) = state
            .mount
            .as_ref()
            .filter(|m| m.label == label)
            .map(|m| m.session.clone())
        else {
            return;
        };
        if matches!(url.scheme(), "http" | "https" | "mailto")
            && let Some(viewer) = state.viewers.get_mut(&session)
        {
            viewer.external = Some(url.as_str().chars().take(2000).collect());
        }
        drop(state);
        tracing::info!(scheme = url.scheme(), "page viewer navigation refused");
        self.0.changed(Some(&session));
    }
}

impl PreviewHooks for ViewerHooks {
    fn allow_navigation(&self, label: &str, url: &Url) -> bool {
        let port = {
            let state = self.0.lock();
            match state.mount.as_ref().filter(|m| m.label == label) {
                Some(m) => m.port,
                None => return false,
            }
        };
        let ok = allowed(url, port);
        if !ok {
            self.note_external(label, url);
        }
        ok
    }

    fn navigation_allowed(&self, label: &str, url: &Url) -> bool {
        let state = self.0.lock();
        state
            .mount
            .as_ref()
            .filter(|m| m.label == label)
            .is_some_and(|m| allowed(url, m.port))
    }

    fn request_allowed(&self, _label: &str, _url: &Url) -> bool {
        // The viewer's pages come from its own page server (CSP `connect-src 'self'`); sub-requests stay as before.
        true
    }

    fn network_guard(&self, _label: &str, _check: vw_preview::GuardCheck) {
        // The page viewer never gets the browser's network guard: it shows only its own loopback page server.
    }

    fn page_event(&self, label: &str, event: PageEvent) -> bool {
        tracing::debug!(label, ?event, "page viewer event");
        let mut guard = self.0.lock();
        let state: &mut State = &mut guard;
        let Some(mount) = state.mount.clone().filter(|m| m.label == label) else {
            tracing::debug!(
                label,
                current = state.mount.as_ref().map_or("-", |m| m.label.as_str()),
                "page viewer event from a webview that is not the mounted one"
            );
            return false;
        };
        let Some(viewer) = state.viewers.get_mut(&mount.session) else {
            tracing::debug!(label, "page viewer event without a viewer for its session");
            return true;
        };
        let mut keep = true;
        let mut finished = false;
        match event {
            PageEvent::Started(url) | PageEvent::Finished(url) | PageEvent::Probe(url)
                if !allowed(&url, mount.port) =>
            {
                viewer.loading = false;
                keep = false;
            }
            PageEvent::Started(url) => {
                viewer.loading = true;
                if url.as_str() != "about:blank" {
                    viewer.path = path_of(&url, self.0.pdf);
                    let path = viewer.path.clone();
                    if let Some(m) = state.mount.as_mut() {
                        m.path = path;
                    }
                }
            }
            PageEvent::Finished(url) => {
                viewer.loading = false;
                if url.as_str() != "about:blank" {
                    viewer.path = path_of(&url, self.0.pdf);
                    viewer.loaded_mtime = viewer
                        .file()
                        .and_then(|f| std::fs::metadata(f).ok())
                        .and_then(|m| m.modified().ok());
                    finished = true;
                }
            }
            PageEvent::Title(title) => viewer.title = Some(title.chars().take(200).collect()),
            PageEvent::TitleAt { text, .. } => {
                viewer.title = Some(text.chars().take(200).collect());
            }
            PageEvent::Probe(_) => return true,
        }
        drop(guard);
        if finished {
            self.0.loads.send_modify(|n| *n += 1);
        }
        self.0.changed(Some(&mount.session));
        keep
    }

    fn new_window_refused(&self, label: &str, url: &Url) {
        self.note_external(label, url);
    }

    fn download_destination(&self, label: &str, url: &Url, _suggested: &Path) -> Option<PathBuf> {
        tracing::info!(
            label,
            scheme = url.scheme(),
            "page viewer refused a download"
        );
        None
    }

    fn download_finished(&self, _label: &str, _url: &Url, _path: Option<&Path>, _ok: bool) {}
}
