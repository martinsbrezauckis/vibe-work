//! The visual check of agent-written handbook pages:
//! before a page shows up, the app lays it out at desktop (1280 px) and phone (390 px) width in the light and the
//! dark theme and reports text that is cut off, sideways scrolling, unreadable colours (WCAG contrast below 4.5:1, 3:1
//! for large text), a page that ignores the app's theme, and a prototype's script errors.
//!
//! How: a hidden webview of its own engine instance (label `vw-preview-check-<n>`, never shown; the app's navigation
//! guard and capability file treat `vw-preview-` labels as untrusted children with no IPC) loads the page server's
//! check document (`/.vw-check-<token>/`, `server`), and the ENGINE evaluates `visual_check.js` there, which lays the
//! page out in two iframes of exact widths, so the result does not depend on the hidden webview's own size or on the
//! OS. Results are cached per page file by the bytes of the page and of `handbook.css`.
//!
//! Depends on: vw-preview (engine seam), `server` (check route), `service` (page servers), `html` (clip), tokio,
//! serde. Called by: vw-core-api (`visualCheck`, the agent's `handbook_visual_check`, the background checks that feed
//! the hub's badges), tests.
//! Invariants:
//! - One check at a time (an async lock); every engine call is bounded; a check that cannot finish answers an
//!   `error`, never hangs.
//! - The checker webview only navigates to a page server this service started (its ports), never elsewhere; new
//!   windows and downloads are refused.
//! - The script's answer is untrusted page data: parsed as JSON only, every field capped again here.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use url::Url;
use vw_preview::{
    EngineError, MountId, MountSpec, PageEvent, PreviewEngine, PreviewHooks, Storage, StorageKey,
};

use crate::config::PagesError;
use crate::html::clip;
use crate::paths::PagePath;
use crate::server::PagesServer;

/// Label prefix of the checker's hidden webview.
pub const CHECK_LABEL_PREFIX: &str = "vw-preview-check-";
/// The issue kinds a report can hold.
pub const ISSUE_KINDS: &[&str] = &["cutOff", "sideways", "contrast", "theme", "scriptError"];
/// Most issues kept per report.
const MAX_ISSUES: usize = 40;
/// Longest answer parsed (bytes).
const MAX_ANSWER: usize = 256 * 1024;
/// The script (placeholders `__ACCESSOR__`, `__PAGE__`, `__WAIT__`).
const SCRIPT: &str = include_str!("visual_check.js");

/// One problem the check found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VisualIssue {
    /// One of [`ISSUE_KINDS`].
    pub kind: String,
    /// Where it shows: `desktop light`, `phone dark`, `desktop`, ...
    pub views: Vec<String>,
    /// A short CSS path of the element (empty for the whole page).
    pub element: String,
    /// The start of the element's text (untrusted page text).
    pub text: String,
    /// What is wrong, plain words with numbers.
    pub detail: String,
}

/// The result of checking one page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VisualReport {
    /// The page (path inside the handbook folder).
    pub page: String,
    /// When it was checked, ms since the Unix epoch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub checked_ms: u64,
    /// The problems found (at most 40).
    pub issues: Vec<VisualIssue>,
    /// Remarks about the check itself (a width that did not load).
    pub notes: Vec<String>,
    /// The check could not run (the reason); `issues` is then empty and says nothing.
    pub error: Option<String>,
}

impl VisualReport {
    /// True when the check ran and found nothing.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.error.is_none() && self.issues.is_empty() && self.notes.is_empty()
    }
}

/// The hooks of the checker's engine: navigation only to this service's page servers.
struct CheckHooks {
    ports: Arc<Mutex<HashSet<u16>>>,
    loads: tokio::sync::watch::Sender<u64>,
}

impl CheckHooks {
    fn ok(&self, url: &Url) -> bool {
        url.as_str() == "about:blank"
            || self
                .ports
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|port| crate::viewer::allowed(url, *port))
    }
}

impl PreviewHooks for CheckHooks {
    fn allow_navigation(&self, _label: &str, url: &Url) -> bool {
        self.ok(url)
    }

    fn navigation_allowed(&self, _label: &str, url: &Url) -> bool {
        self.ok(url)
    }

    fn request_allowed(&self, _label: &str, url: &Url) -> bool {
        // The checker's pages come from the pages server only (its CSP limits
        // their own requests); sub-requests are judged like the page viewer's.
        let _ = url;
        true
    }

    fn network_guard(&self, _label: &str, _check: vw_preview::GuardCheck) {
        // The visual checker never gets the browser's network guard: it shows only the pages server.
    }

    fn page_event(&self, _label: &str, event: PageEvent) -> bool {
        match event {
            PageEvent::Finished(url) if self.ok(&url) => {
                self.loads.send_modify(|n| *n += 1);
                true
            }
            PageEvent::Started(url) | PageEvent::Finished(url) | PageEvent::Probe(url) => {
                self.ok(&url)
            }
            PageEvent::Title(_) | PageEvent::TitleAt { .. } => true,
        }
    }

    fn new_window_refused(&self, _label: &str, _url: &Url) {}

    fn download_destination(&self, _label: &str, _url: &Url, _suggested: &Path) -> Option<PathBuf> {
        None
    }

    fn download_finished(&self, _label: &str, _url: &Url, _path: Option<&Path>, _ok: bool) {}
}

/// The visual checker (one per app).
pub struct VisualChecker {
    engine: Arc<dyn PreviewEngine>,
    ports: Arc<Mutex<HashSet<u16>>>,
    loads: tokio::sync::watch::Sender<u64>,
    storage: PathBuf,
    mount: AsyncMutex<Option<MountId>>,
    generation: Mutex<u64>,
    /// Reports by page file, with the fingerprint (page + stylesheet bytes) they were made from.
    cache: Mutex<HashMap<PathBuf, (u64, VisualReport)>>,
    /// Longest engine call.
    pub engine_timeout: Duration,
    /// Longest wait for one width to load inside the check (ms, passed to the script).
    pub frame_wait: Duration,
    /// Longest whole check.
    pub check_timeout: Duration,
}

impl VisualChecker {
    /// A checker over `engine` (its hooks are attached here); `storage` is the hidden webview's own data folder.
    #[must_use]
    pub fn new(engine: Arc<dyn PreviewEngine>, storage: PathBuf) -> Self {
        let ports = Arc::new(Mutex::new(HashSet::new()));
        let loads = tokio::sync::watch::channel(0).0;
        engine.attach(Arc::new(CheckHooks {
            ports: ports.clone(),
            loads: loads.clone(),
        }));
        Self {
            engine,
            ports,
            loads,
            storage,
            mount: AsyncMutex::new(None),
            generation: Mutex::new(0),
            cache: Mutex::new(HashMap::new()),
            engine_timeout: Duration::from_secs(10),
            frame_wait: Duration::from_secs(8),
            check_timeout: Duration::from_secs(30),
        }
    }

    /// The cached report of `file` when it is still current (same page and stylesheet bytes).
    #[must_use]
    pub fn cached(&self, root: &Path, page: &PagePath) -> Option<VisualReport> {
        let file = page.under(root);
        let print = fingerprint(root, &file)?;
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&file)
            .filter(|(p, _)| *p == print)
            .map(|(_, r)| r.clone())
    }

    /// Checks `page` of the handbook `server` serves (from the cache when the page and stylesheet are unchanged).
    ///
    /// # Errors
    /// [`PagesError::NoPage`]; engine failures before the check could start (an engine without `evaluate`).
    pub async fn check(
        &self,
        server: &PagesServer,
        page: &PagePath,
    ) -> Result<VisualReport, PagesError> {
        let root = server.root().to_path_buf();
        let file = page.under(&root);
        let (owned_root, owned_file) = (root.clone(), file.clone());
        let print = tokio::task::spawn_blocking(move || fingerprint(&owned_root, &owned_file))
            .await
            .map_err(|e| PagesError::Other(e.to_string()))?
            .ok_or_else(|| PagesError::NoPage(page.to_string()))?;
        if let Some(report) = self.cached(&root, page) {
            return Ok(report);
        }
        let report = match tokio::time::timeout(self.check_timeout, self.run(server, page)).await {
            Ok(Ok(report)) => report,
            Ok(Err(PagesError::Engine(EngineError::Unsupported(what)))) => {
                return Err(PagesError::Engine(EngineError::Unsupported(what)));
            }
            Ok(Err(other)) => failed(page, &other.to_string()),
            Err(_) => failed(page, "the check took longer than 30 s"),
        };
        if report.error.is_none() {
            self.cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(file, (print, report.clone()));
        }
        Ok(report)
    }

    async fn run(&self, server: &PagesServer, page: &PagePath) -> Result<VisualReport, PagesError> {
        let mut mount = self.mount.lock().await;
        self.ports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(server.port());
        let host = Url::parse(&format!("{}{}/", server.base_url(), server.check_prefix()))
            .map_err(|e| PagesError::Other(e.to_string()))?;
        let before = *self.loads.borrow();
        let mut loads = self.loads.subscribe();
        let id = if let Some(id) = mount.as_ref() {
            bounded(
                self.engine_timeout,
                "check: navigate",
                self.engine.navigate(id, host.clone()),
            )
            .await?;
            id.clone()
        } else {
            let id = self.mount_new(host.clone()).await?;
            *mount = Some(id.clone());
            id
        };
        if tokio::time::timeout(self.engine_timeout, loads.wait_for(|n| *n > before))
            .await
            .is_err()
        {
            return Ok(failed(page, "the check document did not load"));
        }
        let accessor = accessor();
        let script = SCRIPT
            .replace("__ACCESSOR__", &json_string(&accessor))
            .replace("__PAGE__", &json_string(page.as_str()))
            .replace("__WAIT__", &self.frame_wait.as_millis().to_string());
        let mut answer = bounded(
            self.engine_timeout,
            "check: start",
            self.engine.evaluate(&id, script),
        )
        .await?;
        let poll = format!(
            "(() => {{ const f = window[{}]; return typeof f === 'function' ? f() : '{{\"state\":\"gone\"}}'; }})()",
            json_string(&accessor)
        );
        loop {
            match parse(page, &answer) {
                Parsed::Pending => {}
                Parsed::Done(report) => return Ok(report),
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            answer = bounded(
                self.engine_timeout,
                "check: poll",
                self.engine.evaluate(&id, poll.clone()),
            )
            .await?;
        }
    }

    async fn mount_new(&self, url: Url) -> Result<MountId, PagesError> {
        let label = {
            let mut generation = self
                .generation
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *generation += 1;
            format!("{CHECK_LABEL_PREFIX}{generation}")
        };
        let id = MountId {
            session_id: "page-check".to_owned(),
            label,
        };
        let spec = MountSpec {
            id: id.clone(),
            url,
            storage: Storage {
                key: StorageKey::of("vibe-work-page-check"),
                dir: self.storage.clone(),
            },
            console_script: String::new(),
            console_drain: "'[]'".to_owned(),
            context_menu: false,
            // Never shown: parked hidden by the engine, at its smallest size (the check script lays the page out in
            // iframes of exact widths, so the webview's own size does not matter).
            placement: None,
            parked: None,
            rules: None,
        };
        bounded(self.engine_timeout, "check: mount", self.engine.mount(spec)).await?;
        Ok(id)
    }

    /// Closes the hidden webview (quit).
    pub async fn shutdown(&self) {
        if let Some(id) = self.mount.lock().await.take() {
            let _ = bounded(self.engine_timeout, "check: close", self.engine.close(&id)).await;
        }
    }
}

async fn bounded<T>(
    limit: Duration,
    what: &str,
    future: vw_preview::EngineFuture<'_, T>,
) -> Result<T, PagesError> {
    tokio::time::timeout(limit, future)
        .await
        .map_err(|_| PagesError::Engine(EngineError::Timeout(what.to_owned())))?
        .map_err(PagesError::Engine)
}

fn failed(page: &PagePath, reason: &str) -> VisualReport {
    VisualReport {
        page: page.to_string(),
        checked_ms: now_ms(),
        issues: Vec::new(),
        notes: Vec::new(),
        error: Some(clip(reason, 300)),
    }
}

enum Parsed {
    Pending,
    Done(VisualReport),
}

/// Reads the script's answer (the engine's JSON of the returned string).
fn parse(page: &PagePath, result: &str) -> Parsed {
    if result.len() > MAX_ANSWER {
        return Parsed::Done(failed(page, "the check's answer was too large"));
    }
    let outer: serde_json::Value = serde_json::from_str(result).unwrap_or_default();
    let value = match outer {
        serde_json::Value::String(inner) => serde_json::from_str(&inner).unwrap_or_default(),
        other => other,
    };
    let text = |v: &serde_json::Value, key: &str, max: usize| {
        v.get(key)
            .and_then(serde_json::Value::as_str)
            .map(|s| clip(s, max))
            .unwrap_or_default()
    };
    match value.get("state").and_then(serde_json::Value::as_str) {
        Some("pending") => Parsed::Pending,
        Some("done") => {
            let list = |key: &str| -> Vec<serde_json::Value> {
                value
                    .get(key)
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            };
            let issues = list("issues")
                .iter()
                .filter(|i| ISSUE_KINDS.contains(&text(i, "kind", 20).as_str()))
                .take(MAX_ISSUES)
                .map(|i| VisualIssue {
                    kind: text(i, "kind", 20),
                    views: i
                        .get("views")
                        .and_then(serde_json::Value::as_array)
                        .map(|v| {
                            v.iter()
                                .filter_map(serde_json::Value::as_str)
                                .take(4)
                                .map(|s| clip(s, 30))
                                .collect()
                        })
                        .unwrap_or_default(),
                    element: text(i, "element", 160),
                    text: text(i, "text", 100),
                    detail: text(i, "detail", 300),
                })
                .collect();
            let notes = list("notes")
                .iter()
                .filter_map(serde_json::Value::as_str)
                .take(4)
                .map(|s| clip(s, 200))
                .collect();
            Parsed::Done(VisualReport {
                page: page.to_string(),
                checked_ms: now_ms(),
                issues,
                notes,
                error: None,
            })
        }
        Some("failed") => Parsed::Done(failed(
            page,
            &format!("the check failed: {}", text(&value, "error", 200)),
        )),
        _ => Parsed::Done(failed(page, "the check document went away")),
    }
}

/// A fingerprint of the page file and the handbook's stylesheet (bytes), `None` when the page is not a file.
fn fingerprint(root: &Path, file: &Path) -> Option<u64> {
    use std::hash::{Hash as _, Hasher as _};
    if !file.is_file() || !crate::handbook::files::inside(root, file) {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::fs::read(file).ok()?.hash(&mut hasher);
    std::fs::read(root.join(crate::handbook::format::STYLESHEET))
        .unwrap_or_default()
        .hash(&mut hasher);
    Some(hasher.finish())
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

/// A random accessor name for one check (a collision guard, not a secret).
fn accessor() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        return format!("__vw_check_{}", now_ms());
    }
    format!("__vw_check_{}", crate::comments::hex(&bytes))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
