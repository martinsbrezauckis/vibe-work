//! The `files` command: resolve paths from the chat to files the app may show, read them for the File
//! tab, show HTML pages and PDFs in the isolated viewer, open a file with its default app (never a program) or show
//! it in its folder.
//!
//! Depends on: `files_scope` (which paths), `files_read` (what a file is), `files_view` (wire types), vw-pages (a
//! second `PagesService` = the files viewer, its own webview label), vw-platform (`open_file`, `reveal_file`),
//! vw-gate (picked folders, confirmations), `pages` (the hub type and its event task), `core::Shared`.
//! Called by: `dispatch` (`Command::Files`), `core` (start, shutdown), session end.
//! Invariants:
//! - The scope is built from the core's own records: the session's folder (`Names::cwd_of`), the gate's picked
//!   folders, the home folder. A client never names a folder. The scope protects against agent output and
//!   untrusted file content (a path in the chat that points elsewhere is never clickable, never probed).
//! - The Debug API token holder acts as the user. Both origins get the same scope and
//!   the same answers. "Open with default app" of an HTML or SVG file asks first (a browser runs its scripts);
//!   for the Debug API the core's confirmer answers that itself, without a dialog, and logs it.
//!   `confirmer.rs` is the one choke point for this decision.
//! - Handles (`file`) are random, bound to one session and one canonical path; every use re-checks that path
//!   (`files_scope::recheck`): a file swapped for a symlink out of the folder after `resolve` is refused.
//! - Errors never carry an absolute path.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::json;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer as _};
use vw_pages::PagePath;
use vw_platform::OpenClass;

use crate::command::{CommandResult, Origin, requester};
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::files_read::{FileKind, read_file};
use crate::files_scope::{Hit, Scope, resolve};
use crate::files_view::{FileContent, FileRefView, FilesRequest, FilesResult, MAX_RESOLVE};
use crate::pages::PagesHub;

/// Most handles kept (the oldest is forgotten; the window re-resolves what it still shows).
const MAX_HANDLES: usize = 4096;

/// One handle.
#[derive(Debug, Clone)]
struct Handle {
    session: String,
    path: PathBuf,
}

#[derive(Default)]
struct Handles {
    by_token: HashMap<String, Handle>,
    by_key: HashMap<(String, PathBuf), String>,
    order: VecDeque<String>,
}

impl Handles {
    fn token_for(&mut self, session: &str, path: &Path) -> String {
        let key = (session.to_owned(), path.to_path_buf());
        if let Some(token) = self.by_key.get(&key) {
            return token.clone();
        }
        let token = new_token();
        if self.order.len() >= MAX_HANDLES
            && let Some(old) = self.order.pop_front()
            && let Some(gone) = self.by_token.remove(&old)
        {
            self.by_key.remove(&(gone.session, gone.path));
        }
        self.by_token.insert(
            token.clone(),
            Handle {
                session: session.to_owned(),
                path: path.to_path_buf(),
            },
        );
        self.by_key.insert(key, token.clone());
        self.order.push_back(token.clone());
        token
    }
}

fn new_token() -> String {
    let mut buf = [0_u8; 16];
    // Handles grant nothing by themselves (every use re-checks the scope); a time-based fallback is enough.
    if getrandom::fill(&mut buf).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        buf = nanos.to_le_bytes();
    }
    buf.iter().fold(String::from("f-"), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// The File tab's state in the core.
pub(crate) struct FilesState {
    handles: Mutex<Handles>,
    /// The files viewer (HTML, PDF), when the app gave a viewer engine.
    pub(crate) viewer: Option<Arc<PagesHub>>,
}

impl FilesState {
    pub(crate) fn new(viewer: Option<Arc<PagesHub>>) -> Self {
        Self {
            handles: Mutex::new(Handles::default()),
            viewer,
        }
    }
}

fn result(result: FilesResult) -> CommandResult {
    CommandResult::Files { result }
}

fn gone() -> CoreError {
    CoreError::new(
        ErrorCode::NotFound,
        "this file is not there any more, or it may not be shown",
    )
}

/// How a hit is named: relative to the session folder, else its full path (a picked folder) in its everyday
/// spelling (Windows: `C:\x`, never the canonical `\\?\C:\x`).
fn display(hit: &Hit) -> String {
    if hit.in_session
        && let Ok(rest) = hit.canonical.strip_prefix(&hit.root)
    {
        return rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
    }
    vw_platform::plain_path(&hit.canonical)
        .display()
        .to_string()
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("file task failed: {e}")))
}

impl Core {
    /// The scope of `session_id` (see `files_scope`): the same for the window and the Debug API.
    fn files_scope(&self, session_id: &str) -> Result<Scope, CoreError> {
        Ok(Scope {
            session: self.project_of(session_id)?,
            picked: self.shared.gate.picked_roots(),
            home: home(),
            hidden: true,
        })
    }

    /// The handle's file, checked again against the session's scope.
    pub(crate) async fn checked(&self, session_id: &str, file: &str) -> Result<Hit, CoreError> {
        let handle = self
            .shared
            .files
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .by_token
            .get(file)
            .cloned()
            .filter(|h| h.session == session_id)
            .ok_or_else(gone)?;
        let scope = self.files_scope(session_id)?;
        let path = handle.path;
        blocking(move || crate::files_scope::recheck(&scope, &path))
            .await?
            .ok_or_else(gone)
    }

    /// Executes one File tab request for `origin`.
    pub(crate) async fn files(
        &self,
        origin: Origin,
        request: FilesRequest,
    ) -> Result<CommandResult, CoreError> {
        match request {
            FilesRequest::Resolve { session_id, paths } => {
                self.files_resolve(&session_id, paths).await
            }
            FilesRequest::Read { session_id, file } => {
                let hit = self.checked(&session_id, &file).await?;
                let path = hit.canonical.clone();
                let read = blocking(move || read_file(&path))
                    .await?
                    .map_err(|_| gone())?;
                let probe = hit.canonical.clone();
                let class = blocking(move || vw_platform::file_open_class(&probe))
                    .await?
                    .map_or(OpenClass::Refused, |(_, c)| c);
                Ok(result(FilesResult::Content {
                    content: FileContent {
                        display: display(&hit),
                        name: name_of(&hit.canonical),
                        file,
                        kind: read.kind,
                        size: read.size,
                        fingerprint: read.fingerprint,
                        text: read.text,
                        truncated: read.truncated,
                        data_url: read.data_url,
                        too_large: read.too_large,
                        open_class: class_name(class).to_owned(),
                    },
                }))
            }
            FilesRequest::Stat { session_id, file } => {
                let found = self.checked(&session_id, &file).await.ok();
                // The same fingerprint a read gives (content hash included for files small enough to edit), so the
                // tab's poll notices a rewrite that kept the size and the modification time.
                let fingerprint = match found {
                    Some(hit) => {
                        let path = hit.canonical.clone();
                        crate::pages::blocking(move || {
                            crate::files_read::fingerprint_of(&path).ok()
                        })
                        .await?
                    }
                    None => None,
                };
                Ok(result(FilesResult::Stat {
                    exists: fingerprint.is_some(),
                    fingerprint,
                }))
            }
            FilesRequest::Open { session_id, file } => {
                self.files_open(origin, &session_id, &file, false).await
            }
            FilesRequest::Reveal { session_id, file } => {
                self.files_open(origin, &session_id, &file, true).await
            }
            FilesRequest::Write {
                session_id,
                file,
                text,
                fingerprint,
            } => {
                let (fingerprint, size) = self
                    .files_write(&session_id, &file, text, &fingerprint)
                    .await?;
                Ok(result(FilesResult::Written { fingerprint, size }))
            }
            other => self.files_viewer(other).await,
        }
    }

    async fn files_resolve(
        &self,
        session_id: &str,
        paths: Vec<String>,
    ) -> Result<CommandResult, CoreError> {
        if paths.len() > MAX_RESOLVE {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                format!("at most {MAX_RESOLVE} paths per request"),
            ));
        }
        let scope = self.files_scope(session_id)?;
        let hits = blocking(move || {
            paths
                .into_iter()
                .map(|text| {
                    let hit = resolve(&scope, &text);
                    (text, hit)
                })
                .collect::<Vec<_>>()
        })
        .await?;
        let mut handles = self
            .shared
            .files
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let refs = hits
            .into_iter()
            .map(|(text, hit)| match hit {
                Some(hit) => FileRefView {
                    file: Some(handles.token_for(session_id, &hit.canonical)),
                    display: Some(display(&hit)),
                    kind: Some(FileKind::guess(&name_of(&hit.canonical))),
                    size: Some(hit.size),
                    ok: true,
                    text,
                },
                None => FileRefView {
                    text,
                    ok: false,
                    file: None,
                    display: None,
                    kind: None,
                    size: None,
                },
            })
            .collect();
        Ok(result(FilesResult::Resolved { refs }))
    }

    /// `open` (default app) or `reveal` (file manager).
    async fn files_open(
        &self,
        origin: Origin,
        session_id: &str,
        file: &str,
        reveal: bool,
    ) -> Result<CommandResult, CoreError> {
        let hit = self.checked(session_id, file).await?;
        let probe = hit.canonical.clone();
        let class = if reveal {
            OpenClass::Passive
        } else {
            blocking(move || vw_platform::file_open_class(&probe))
                .await?
                .map_err(|_| gone())?
                .1
        };
        if class == OpenClass::Refused {
            return Err(CoreError::new(
                ErrorCode::Denied,
                vw_platform::OpenFileError::NotOpenable.to_string(),
            ));
        }
        // An HTML/SVG file asks first. The confirmer answers a Debug API request itself, without a dialog, and logs
        // it (the Debug API acts as the user; one choke point, `confirmer.rs`).
        if class == OpenClass::Active && !reveal {
            let name = name_of(&hit.canonical);
            let method = "files/open";
            let request = ConfirmationRequest {
                method: method.to_owned(),
                requester: requester(origin),
                summary: format!(
                    "Open {} from session {} with your default app? A browser runs its scripts with network access.",
                    vw_gate::dialog_quote(&name),
                    vw_gate::dialog_quote(session_id),
                ),
                params: json!({ "name": name }),
                always: None,
            };
            if self.shared.confirmer.confirm(&request).await != Confirmation::Approved {
                let mut error = CoreError::new(
                    ErrorCode::ConfirmationDeclined,
                    "the request was not confirmed in the native dialog",
                );
                error.method = Some(method.to_owned());
                return Err(error);
            }
        }
        let path = hit.canonical;
        blocking(move || {
            if reveal {
                vw_platform::reveal_file(&path)
            } else {
                vw_platform::open_file(&path, true)
            }
        })
        .await?
        .map_err(|e| {
            let code = match e {
                vw_platform::OpenFileError::Spawn(_) => ErrorCode::Internal,
                _ => ErrorCode::Denied,
            };
            CoreError::new(code, e.to_string())
        })?;
        Ok(result(FilesResult::Done {}))
    }

    /// The viewer requests (HTML pages and PDFs in the isolated webview).
    async fn files_viewer(&self, request: FilesRequest) -> Result<CommandResult, CoreError> {
        let service = self
            .shared
            .files
            .viewer
            .as_ref()
            .map(|hub| hub.service.clone())
            .ok_or_else(|| {
                CoreError::not_in_this_build(
                    "the page viewer is not available in this build of Vibe Work",
                )
            })?;
        let err = |e: vw_pages::PagesError| crate::pages::core_error(&e);
        let viewer = match request {
            FilesRequest::View { session_id, file } => {
                let hit = self.checked(&session_id, &file).await?;
                let root = hit
                    .canonical
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(gone)?;
                let page = PagePath::viewable(&name_of(&hit.canonical)).map_err(|_| {
                    CoreError::new(
                        ErrorCode::InvalidParams,
                        "only HTML pages and PDFs with a plain (not hidden) name can be shown in the viewer",
                    )
                })?;
                Some(
                    service
                        .open_file(&session_id, &root, page)
                        .await
                        .map_err(err)?,
                )
            }
            FilesRequest::Layout { session_id, rect } => {
                service.layout(session_id.zip(rect)).await.map_err(err)?;
                return Ok(result(FilesResult::Done {}));
            }
            FilesRequest::Reload { session_id } => {
                service
                    .history(&session_id, vw_preview::HistoryStep::Reload)
                    .await
                    .map_err(err)?;
                service.view(&session_id)
            }
            FilesRequest::Viewer { session_id } => service.view(&session_id),
            FilesRequest::ClearExternal { session_id } => {
                service.clear_external(&session_id);
                service.view(&session_id)
            }
            FilesRequest::Screenshot { session_id } => {
                let shot = service.screenshot(&session_id).await.map_err(err)?;
                return Ok(result(FilesResult::Screenshot {
                    png_base64: shot.base64(),
                    width: shot.width,
                    height: shot.height,
                }));
            }
            _ => {
                return Err(CoreError::new(
                    ErrorCode::Internal,
                    "files request without a handler",
                ));
            }
        };
        Ok(result(FilesResult::Viewer { viewer }))
    }
}

fn class_name(class: OpenClass) -> &'static str {
    match class {
        OpenClass::Passive => "passive",
        OpenClass::Active => "active",
        OpenClass::Refused => "refused",
    }
}

/// A session ended: its viewer goes.
pub(crate) async fn session_ended(shared: &Shared, session_id: &str) {
    if let Some(hub) = &shared.files.viewer {
        hub.service.session_ended(session_id).await;
    }
}

/// Closes the viewer and stops its page servers (quit).
pub(crate) async fn shutdown(shared: &Shared) {
    if let Some(hub) = &shared.files.viewer {
        hub.stop_task();
        hub.service.shutdown().await;
    }
}

/// Starts forwarding the files viewer's changes as `filesChanged` events and its reload tick.
pub(crate) fn start_events(shared: &Arc<Shared>) {
    if let Some(hub) = &shared.files.viewer {
        crate::pages::start_hub_events(shared, hub, |session_id| CoreEvent::FilesChanged {
            session_id,
        });
    }
}
