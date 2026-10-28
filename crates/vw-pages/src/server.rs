//! The page server: one loopback HTTP server per project handbook folder, the origin the page viewer loads pages
//! from (relative links and assets work like on a web server). Adapted from ShellX `work_preview.rs`
//! `start_static` / `static_preview_handler` / `static_preview_file_for_uri`:
//! the per-segment checks and the canonical-root rule are kept; ShellX's injected "doctor" script is not (pages
//! are served byte-identical), and every answer carries a Content-Security-Policy (ShellX sent none).
//!
//! Depends on: axum 0.8 (HTTP/1.1), tokio, `paths`, `markdown`, `hub`. Called by: `service`, tests.
//! Invariants:
//! - Binds `127.0.0.1:0` only; a request whose `Host` is not this server's own `127.0.0.1:<port>` or
//!   `localhost:<port>` is refused (421): another site cannot reach it through DNS rebinding.
//! - Every content route starts with a capability (32 random bytes per server, hex encoded); a missing or wrong
//!   first segment is refused (404) before reading any file. Relative links and assets keep that segment.
//! - Serves only regular files whose canonical path lies under the canonical pages folder, every segment of the
//!   request and of the resolved path passing `paths::segment_allowed` (no hidden files, no `..`), with an
//!   extension on the positive list (`paths::asset_allowed`; plus `.pdf` for the File tab's servers), at
//!   most [`MAX_FILE`] bytes. Never lists a folder.
//! - Every answer: `Content-Security-Policy` (no network beyond this server, no forms, no plugins, no `<base>`),
//!   `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cache-Control: no-store`.
//! - Handbook servers (Format 1.2, CSP per kind, [`Policy`]): only a page of kind `prototype` may run script (its own
//!   inline and same-origin scripts, unless the project turned prototype scripts off), and it runs sandboxed
//!   (`sandbox allow-scripts`: an opaque origin, so no storage and no reading of other pages) with `connect-src
//!   'none'` and `frame-src 'none'`; every other page, Markdown page and the live index get `script-src 'none'`.
//!   The File tab's servers keep one policy for every HTML file ([`csp`], scripts on).
//! - The visual check's route (`/<capability>/.vw-check-<token>/...`, [`PagesServer::check_prefix`]) serves the same
//!   files to the hidden checker webview only: a prototype page there runs WITHOUT the sandbox (the check document must measure
//!   it from the same origin) and with a small error-recording script put after its doctype ([`ERROR_PRELUDE`]);
//!   still no network, and the checker webview has no IPC. Its route root is an empty check document. The
//!   token (128 random bits per server) is known only to the core and the checker: a page in the viewer cannot name
//!   the route (a normal page framing the unsandboxed check copy of a prototype would
//!   have given that prototype same-origin access to other pages through its parent). Normal pages may not frame
//!   anything (`frame-src 'none'`), and every answer carries `X-DNS-Prefetch-Control: off`.
//! - Nothing is written: the live index is built in memory for each request (Handbook Format 1.2 §8).
//! - GET and HEAD only.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use crate::csp::{CHECK_PREFIX, ERROR_PRELUDE, Policy, csp};
use crate::paths::{INDEX, asset_allowed, segment_allowed};

/// Largest file served.
pub const MAX_FILE: u64 = 20 * 1024 * 1024;

struct ServerState {
    root: PathBuf,
    capability: String,
    /// `.vw-check-<128 random bits hex>`: the check route's first segment (see the module header).
    check: String,
    /// `.pdf` is served (the File tab's servers).
    pdf: bool,
    port: u16,
    scripts: Arc<AtomicBool>,
    project: String,
}

/// A running page server; stops when dropped.
pub struct PagesServer {
    port: u16,
    root: PathBuf,
    capability: String,
    scripts: Arc<AtomicBool>,
    check: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for PagesServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PagesServer")
            .field("port", &self.port)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl PagesServer {
    /// Starts a server for the pages folder `root` (it may not exist yet: requests then answer 404 or the empty
    /// hub). `project` names the project in the generated hub. Must be called inside a Tokio runtime.
    ///
    /// # Errors
    /// The bind or random-source error.
    pub async fn start(root: PathBuf, scripts: bool, project: String) -> std::io::Result<Self> {
        Self::start_inner(root, scripts, project, false).await
    }

    /// [`Self::start`] for the File tab's viewer: also serves `.pdf` as `application/pdf` (the engine's
    /// own PDF viewer shows it in the isolated webview).
    ///
    /// # Errors
    /// The bind or random-source error.
    pub async fn start_for_files(root: PathBuf, project: String) -> std::io::Result<Self> {
        Self::start_inner(root, true, project, true).await
    }

    async fn start_inner(
        root: PathBuf,
        scripts: bool,
        project: String,
        pdf: bool,
    ) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let scripts = Arc::new(AtomicBool::new(scripts));
        let mut token = [0u8; 16];
        getrandom::fill(&mut token).map_err(|e| std::io::Error::other(e.to_string()))?;
        let check = format!("{CHECK_PREFIX}{}", crate::comments::hex(&token));
        let mut capability = [0u8; 32];
        getrandom::fill(&mut capability).map_err(|e| std::io::Error::other(e.to_string()))?;
        let capability = crate::comments::hex(&capability);
        let state = Arc::new(ServerState {
            capability: capability.clone(),
            check: check.clone(),
            root: root.clone(),
            pdf,
            port,
            scripts: scripts.clone(),
            project,
        });
        let router = Router::new().fallback(handle).with_state(state);
        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let served = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
            if let Err(error) = served {
                tracing::warn!(%error, "page server stopped");
            }
        });
        Ok(Self {
            port,
            root,
            capability,
            scripts,
            check,
            shutdown: Some(tx),
            task,
        })
    }

    /// The port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The pages folder it serves.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Turns page scripts on or off for the next answers.
    pub fn set_scripts(&self, on: bool) {
        self.scripts.store(on, Ordering::Relaxed);
    }

    /// The first path segment of this server's visual check route (`.vw-check-<token>`; see the module header).
    /// Only the core and the checker webview may know it.
    #[must_use]
    pub fn check_prefix(&self) -> &str {
        &self.check
    }

    /// The origin (`http://127.0.0.1:<port>`).
    #[must_use]
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The content base URL, including the capability and trailing slash. Only the core and its viewer need it.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("{}/{}/", self.origin(), self.capability)
    }
}

impl Drop for PagesServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        self.task.abort();
    }
}

fn plain(status: StatusCode, origin: &str) -> Response {
    with_headers(
        Response::new(Body::from(status.canonical_reason().unwrap_or("error"))),
        status,
        "text/plain; charset=utf-8",
        &Policy::Document.header(origin),
    )
}

fn with_headers(
    mut response: Response,
    status: StatusCode,
    mime: &'static str,
    csp: &str,
) -> Response {
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    if let Ok(value) = HeaderValue::from_str(csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // DNS prefetching is not governed by the CSP: off, so a page cannot leak a host name through a lookup.
    headers.insert(
        header::HeaderName::from_static("x-dns-prefetch-control"),
        HeaderValue::from_static("off"),
    );
    response
}

/// True when `Host` names this server.
fn host_ok(headers: &HeaderMap, port: u16) -> bool {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|host| {
            host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
        })
}

async fn handle(
    State(state): State<Arc<ServerState>>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let origin = format!("http://127.0.0.1:{}", state.port);
    if method != Method::GET && method != Method::HEAD {
        return plain(StatusCode::METHOD_NOT_ALLOWED, &origin);
    }
    if !host_ok(&headers, state.port) {
        return plain(StatusCode::MISDIRECTED_REQUEST, &origin);
    }
    let origin = if headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| h.starts_with("localhost:"))
    {
        format!("http://localhost:{}", state.port)
    } else {
        origin
    };
    let Some((capability, path)) = uri.path().strip_prefix('/').and_then(|p| p.split_once('/'))
    else {
        return plain(StatusCode::NOT_FOUND, &origin);
    };
    if !bool::from(capability.as_bytes().ct_eq(state.capability.as_bytes())) {
        return plain(StatusCode::NOT_FOUND, &origin);
    }
    let path = format!("/{path}");
    let inline_body = uri.query() == Some("hb-body");
    let worker = state.clone();
    let answer = tokio::task::spawn_blocking(move || serve(&worker, &path, inline_body)).await;
    let (mime, body, policy) = match answer {
        Ok(Ok(served)) => served,
        Ok(Err(status)) => return plain(status, &origin),
        Err(_) => return plain(StatusCode::INTERNAL_SERVER_ERROR, &origin),
    };
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(body)
    };
    let policy = if policy == Policy::InlineHost {
        crate::inline::host_policy(&origin, uri.path())
    } else {
        policy.header(&origin)
    };
    with_headers(Response::new(body), StatusCode::OK, mime, &policy).into_response()
}

/// The empty document the visual check lays pages out in (no script of its own: the engine's script measures).
const CHECK_DOCUMENT: &str = "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Vibe Work page \
check</title><style>html,body{margin:0;background:#808080}iframe{display:block;border:0;background:#fff}</style>\
</head><body></body></html>\n";

/// What one request answers: MIME, bytes and the policy of the answer.
fn serve(
    state: &ServerState,
    uri_path: &str,
    inline_body: bool,
) -> Result<(&'static str, Vec<u8>, Policy), StatusCode> {
    let (check, uri_path) = match uri_path
        .strip_prefix('/')
        .and_then(|p| p.strip_prefix(state.check.as_str()))
    {
        // The check route exists on handbook servers only.
        Some(rest) if !state.pdf && (rest.is_empty() || rest.starts_with('/')) => (true, rest),
        _ => (false, uri_path),
    };
    let segments = request_segments(uri_path)?;
    if check && segments.is_empty() {
        return Ok((
            "text/html; charset=utf-8",
            CHECK_DOCUMENT.as_bytes().to_vec(),
            Policy::CheckHost,
        ));
    }
    let root = &state.root;
    let scripts = state.scripts.load(Ordering::Relaxed);
    if segments.is_empty() && !root.join(INDEX).is_file() {
        let folder = crate::handbook::Folder {
            relative: String::new(),
        };
        let hub = crate::hub::build(root, &folder);
        return Ok((
            "text/html; charset=utf-8",
            crate::hub::live_index_html(&hub, &state.project).into_bytes(),
            Policy::Document,
        ));
    }
    let file = resolve_file(root, &segments)?;
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let pdf = state.pdf
        && Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    let mime = asset_allowed(&name)
        .or(pdf.then_some("application/pdf"))
        .ok_or(StatusCode::NOT_FOUND)?;
    let bytes = read_capped(&file)?;
    if name.to_ascii_lowercase().ends_with(".md") {
        let text = String::from_utf8_lossy(&bytes);
        // The handbook's own stylesheet frames a Markdown page like an HTML page (§7), relative to its folder.
        let stylesheet = root
            .join(crate::handbook::format::STYLESHEET)
            .is_file()
            .then(|| {
                "../".repeat(segments.len().saturating_sub(1)) + crate::handbook::format::STYLESHEET
            });
        return Ok((
            mime,
            crate::markdown::render_at(&text, &name, stylesheet.as_deref(), &segments.join("/"))
                .into_bytes(),
            Policy::Document,
        ));
    }
    if state.pdf {
        // The File tab: one policy for every file of the session's folder.
        return Ok((mime, bytes, Policy::Files { scripts }));
    }
    if !mime.starts_with("text/html") {
        return Ok((mime, bytes, Policy::Document));
    }
    let facts = crate::page::parse_html(&String::from_utf8_lossy(&bytes));
    let prototype =
        !facts.generated && facts.kind.as_deref() == Some(crate::handbook::format::PROTOTYPE);
    if prototype && !check && !facts.inputs.is_empty() {
        if !scripts {
            return Ok((mime, bytes, Policy::Document));
        }
        if inline_body {
            return Ok((
                mime,
                with_script(&bytes, &crate::inline::prelude()),
                Policy::Prototype { scripts },
            ));
        }
        return Ok((
            mime,
            crate::inline::prototype_host(
                &String::from_utf8_lossy(&bytes),
                facts.title.as_deref().unwrap_or("Handbook prototype"),
            ),
            Policy::InlineHost,
        ));
    }
    Ok(match (prototype, check) {
        (false, _) => (mime, bytes, Policy::Document),
        (true, false) => (mime, bytes, Policy::Prototype { scripts }),
        (true, true) => (
            mime,
            with_prelude(&bytes),
            Policy::CheckPrototype { scripts },
        ),
    })
}

/// `page` with [`ERROR_PRELUDE`] right after its doctype (before it would switch the page to quirks mode), else at
/// the very start.
fn with_prelude(page: &[u8]) -> Vec<u8> {
    with_script(page, ERROR_PRELUDE)
}

fn with_script(page: &[u8], prelude: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(page);
    let trimmed = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    let lead = text.len() - trimmed.len();
    let at = if trimmed
        .get(..9)
        .is_some_and(|s| s.eq_ignore_ascii_case("<!doctype"))
    {
        trimmed.find('>').map_or(lead, |end| lead + end + 1)
    } else {
        lead
    };
    let mut out = String::with_capacity(text.len() + prelude.len());
    out.push_str(&text[..at]);
    out.push_str(prelude);
    out.push_str(&text[at..]);
    out.into_bytes()
}

/// Decodes and checks the request path's segments (empty list = the folder itself).
fn request_segments(uri_path: &str) -> Result<Vec<String>, StatusCode> {
    let decoded = strict_decode(uri_path).ok_or(StatusCode::BAD_REQUEST)?;
    let mut segments = Vec::new();
    for part in decoded.split('/') {
        if part.is_empty() {
            continue;
        }
        if !segment_allowed(part) {
            return Err(StatusCode::NOT_FOUND);
        }
        segments.push(part.to_owned());
    }
    if segments.len() > crate::paths::MAX_DEPTH + 1 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(segments)
}

/// The canonical file for `segments` under `root` (a folder answers its `index.html`).
fn resolve_file(root: &Path, segments: &[String]) -> Result<PathBuf, StatusCode> {
    let root_canonical = root.canonicalize().map_err(|_| StatusCode::NOT_FOUND)?;
    let candidate = segments.iter().fold(root.to_path_buf(), |p, s| p.join(s));
    let mut canonical = candidate
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if canonical.is_dir() {
        canonical = canonical
            .join(INDEX)
            .canonicalize()
            .map_err(|_| StatusCode::NOT_FOUND)?;
    }
    let relative = canonical
        .strip_prefix(&root_canonical)
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let clean = relative.components().all(|c| match c {
        std::path::Component::Normal(part) => part.to_str().is_some_and(segment_allowed),
        _ => false,
    });
    if !clean || relative.as_os_str().is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(canonical)
}

/// Reads a regular file of at most [`MAX_FILE`] bytes (checked on the opened file, not the path).
fn read_capped(file: &Path) -> Result<Vec<u8>, StatusCode> {
    use std::io::Read;
    let handle = std::fs::File::open(file).map_err(|_| StatusCode::NOT_FOUND)?;
    let meta = handle.metadata().map_err(|_| StatusCode::NOT_FOUND)?;
    if !meta.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }
    if meta.len() > MAX_FILE {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let mut bytes = Vec::new();
    handle
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok(bytes)
}

/// Percent-decodes a path; a malformed escape or invalid UTF-8 is `None` (answered 400).
fn strict_decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = char::from(*bytes.get(i + 1)?).to_digit(16)?;
            let lo = char::from(*bytes.get(i + 2)?).to_digit(16)?;
            out.push(u8::try_from(hi * 16 + lo).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
