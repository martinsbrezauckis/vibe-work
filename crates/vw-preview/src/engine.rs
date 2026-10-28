//! The seam between the preview model (this crate, driven by the core) and the native webview engine (the Tauri
//! implementation in vw-webview; a recording fake in tests). The core never sees Tauri types.
//!
//! Depends on: url, `layout`, `console`. Called by: `service` (drives an engine), vw-webview (implements it),
//! tests (fakes).
//! Invariants (every engine):
//! - A mounted webview has no IPC permissions and no capability; it is never the main webview and cannot navigate
//!   the main window.
//! - Every navigation, new-window request and download is asked of [`PreviewHooks`]; a refusal is final.
//! - Every operation finishes or fails within its own timeout (none waits forever on the main thread).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use url::Url;

use crate::accounts::{Provider, ProviderAccount};
use crate::console::ConsoleEntry;
use crate::ids::StorageKey;
use crate::layout::Placement;

/// A boxed engine future.
pub type EngineFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, EngineError>> + Send + 'a>>;

/// Why an engine operation failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// The webview is not mounted (closed, suspended, never opened).
    #[error("the preview is not running")]
    NotMounted,
    /// The platform cannot do this (the text says what and where).
    #[error("{0}")]
    Unsupported(String),
    /// The operation did not finish in time.
    #[error("the preview did not answer in time ({0})")]
    Timeout(String),
    /// The platform reported an error.
    #[error("{0}")]
    Platform(String),
    /// Native input is safe only after the user brings the app's window to the front.
    #[error("native preview needs window focus")]
    FocusRequired,
}

/// Identifies one mount of one session's preview.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MountId {
    /// The session.
    pub session_id: String,
    /// The webview label (`StorageKey::label`).
    pub label: String,
}

/// Where a session's preview keeps its website data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    /// The session or shared-browser key (macOS data store identifier).
    pub key: StorageKey,
    /// The session or shared-browser data folder (Windows user data folder, `WebKitGTK` base).
    pub dir: PathBuf,
}

impl Storage {
    /// Whether this is the dedicated public-browser profile shared across sessions.
    #[must_use]
    pub fn is_shared_web(&self) -> bool {
        self.key == StorageKey::shared_web()
    }
}

/// What to mount.
#[derive(Clone, Debug)]
pub struct MountSpec {
    /// Identity.
    pub id: MountId,
    /// First URL (already checked by the policy).
    pub url: Url,
    /// Storage.
    pub storage: Storage,
    /// Console initialization script (`console::init_script`).
    pub console_script: String,
    /// Console drain expression (`console::drain_script`).
    pub console_drain: String,
    /// Whether this is the interactive browser mount with a physical page context menu.
    pub context_menu: bool,
    /// Where to place it right away; `None` = mounted hidden.
    pub placement: Option<Placement>,
    /// Hidden mounts only: the size (and zoom) it is parked at while hidden (`layout::parked`: what it would get if
    /// shown now), so its page lays out at a real viewport. `None` = the engine's smallest size (the page
    /// checker, whose script sets its own widths).
    pub parked: Option<Placement>,
    /// The addresses this mount's page may reach, for an engine that filters requests by URL (macOS
    /// content rules). `None`: the engine adds no URL rules (the page viewer and checker reach only their own
    /// server through their navigation hooks).
    pub rules: Option<vw_netguard::rules::RulePage>,
    /// Remove WebRTC from every frame before page scripts run ([`crate::NO_PEER_CONNECTIONS_SCRIPT`]): the page
    /// viewer and the page checker (a prototype page has no network by its CSP, which does not govern WebRTC).
    /// `false`: the engine leaves WebRTC as the platform has it (previews: the user's own apps may use it).
    pub no_peer_connections: bool,
}

/// A protection layer an engine put on a mount, reported for diagnostics and tests. The navigation
/// policy applies to every mount and is not listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuardLayer {
    /// Every request is decided per webview before it is sent (`WebView2` `WebResourceRequested`).
    RequestBarrier,
    /// Every request goes through the egress guard, proven by the canaries (it resolves names itself and connects to
    /// exactly the checked addresses).
    EgressProxy,
    /// URL content rules keep sub-requests to the mount's addresses (macOS `WKContentRuleList`).
    ContentRules,
}

impl GuardLayer {
    /// The layer's wire name (`requestBarrier`, `egressProxy`, `contentRules`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::RequestBarrier => "requestBarrier",
            Self::EgressProxy => "egressProxy",
            Self::ContentRules => "contentRules",
        }
    }
}

/// A history step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryStep {
    /// Back one entry.
    Back,
    /// Forward one entry.
    Forward,
    /// Reload the page.
    Reload,
}

/// Page lifecycle events an engine reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageEvent {
    /// Check a completed URL's navigation policy before reading its document, without changing load state.
    Probe(Url),
    /// A load started.
    Started(Url),
    /// A load finished.
    Finished(Url),
    /// The document title changed.
    Title(String),
    /// A document title with the URL of the document that reported it.
    TitleAt {
        /// The document URL.
        url: Url,
        /// The title text.
        text: String,
    },
}

/// One step of a mount's network-guard check (`PreviewHooks::network_guard`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardCheck {
    /// The engine is about to load this loopback canary URL in the mount.
    Expect(Url),
    /// The check finished: the guard saw both canaries (`true`: the egress proxy layer) or not.
    Done(bool),
    /// The engine installed another layer on the mount (the request barrier, content rules).
    Layer(GuardLayer),
}

/// A request decision that may wait for a host-name lookup (a lookup never
/// blocks the engine's UI thread).
pub enum RequestCheck {
    /// Send it.
    Allow,
    /// Refuse it (answer `403` without sending it).
    Deny,
    /// Decided when the lookup ends (at most `names::LOOKUP_LIMIT`): call it with the callback that receives the
    /// verdict (`true`: send it). The callback runs on a helper thread (or at once, when the answer is cached); the
    /// engine holds the request meanwhile (`WebView2`: a deferral completed back on the UI thread).
    Later(LaterCheck),
}

/// Receives a held request's verdict (`true`: send it).
pub type CheckDone = Box<dyn FnOnce(bool) + Send>;

/// Registers the callback a held request's verdict goes to ([`RequestCheck::Later`]).
pub type LaterCheck = Box<dyn FnOnce(CheckDone) + Send>;

impl std::fmt::Debug for RequestCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Allow => "Allow",
            Self::Deny => "Deny",
            Self::Later(_) => "Later",
        })
    }
}

/// Callbacks from the engine into the model. Called on the engine's thread (the UI thread for Tauri): every
/// method must return quickly and never block on the async runtime.
pub trait PreviewHooks: Send + Sync {
    /// May the mount `label` navigate to `url`? Refused navigations never happen. Records a refusal.
    fn allow_navigation(&self, label: &str, url: &Url) -> bool;
    /// A frame navigation: apply the same policy, but never replace or cancel a held top-level URL.
    fn allow_frame_navigation(&self, label: &str, url: &Url) -> bool {
        self.allow_navigation(label, url)
    }
    /// The same decision as [`Self::allow_navigation`] without recording anything: asked by an engine whose
    /// navigation callback is not a request barrier (`WebView2` sends the GET of a cancelled navigation), for every
    /// document request (main frame and frames) before it is sent.
    fn navigation_allowed(&self, label: &str, url: &Url) -> bool;
    /// The network-guard check of mount `label`: first `Expect(url)` for the canary the engine is about
    /// to load (allowed until the check ends, never shown), then `Done(active)`, which is only reported (public pages
    /// load either way under the public-page policy). `Layer(..)` reports another layer the engine installed,
    /// such as content rules. An engine without a guard never calls it.
    fn network_guard(&self, label: &str, check: GuardCheck);
    /// May mount `label`'s page send a sub-request (script, image, `fetch`, ...) to `url`? Asked by an engine
    /// whose traffic does not pass the network guard (`WebView2`), before the request is sent: loopback
    /// by owner, private literals only from the Lan page's own host, host names through the cached resolver (fail
    /// closed). Never records anything.
    fn request_allowed(&self, label: &str, url: &Url) -> bool;
    /// [`Self::navigation_allowed`] (`document`) or [`Self::request_allowed`] for an engine that can hold a request:
    /// a host name whose lookup is still running answers [`RequestCheck::Later`] instead of waiting on the UI thread
    /// (`WebView2`'s barrier). A refused document is recorded for the receipts.
    fn request_check(&self, label: &str, url: &Url, document: bool) -> RequestCheck {
        let ok = if document {
            self.navigation_allowed(label, url)
        } else {
            self.request_allowed(label, url)
        };
        if ok {
            RequestCheck::Allow
        } else {
            RequestCheck::Deny
        }
    }
    /// The engine checks host names of mount `label` again at request time and holds or refuses what it cannot
    /// check yet (the Linux egress proxy, the `WebView2` barrier): the navigation hook then lets a navigation whose
    /// name lookup is still running go on instead of refusing it. Called once, before the first navigation. A guard
    /// check that ends unproven (`network_guard(Done(false))`) withdraws it (fail closed).
    fn names_checked_later(&self, label: &str) {
        let _ = label;
    }
    /// A page event of mount `label`. Returns `false` when the page must be unloaded at once: it ended up on a URL
    /// outside the allowed set without a navigation the engine asked about (a redirect path an engine does not
    /// report). The engine then loads `about:blank`.
    fn page_event(&self, label: &str, event: PageEvent) -> bool;
    /// A page asked for a new window (always refused by the engine); for the record.
    fn new_window_refused(&self, label: &str, url: &Url);
    /// A download starts: the destination (always a new file in the app's private staging folder, never the
    /// session folder), or `None` to cancel it.
    fn download_destination(&self, label: &str, url: &Url, suggested: &Path) -> Option<PathBuf>;
    /// A download ended (`ok` = written completely, as far as the engine knows).
    fn download_finished(&self, label: &str, url: &Url, path: Option<&Path>, ok: bool);
}

/// The native engine.
pub trait PreviewEngine: Send + Sync {
    /// Host-authorized native mouse input. Must target only this visible webview in the app's active window.
    /// Default refusal: permission never silently falls back to a synthetic click.
    fn native_click(
        &self,
        id: &MountId,
        point: crate::NativeClickPoint,
        guard: crate::NativeClickGuard,
    ) -> EngineFuture<'_, ()> {
        let _ = (id, point, guard);
        Box::pin(async {
            Err(EngineError::Unsupported(
                "native clicks are unavailable in this preview engine".to_owned(),
            ))
        })
    }
    /// Gives the engine its hooks (called once, before the first mount).
    fn attach(&self, hooks: Arc<dyn PreviewHooks>);
    /// Mounts a webview (no IPC, own storage) and loads `spec.url`.
    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()>;
    /// Loads `url` in a mounted webview.
    fn navigate(&self, id: &MountId, url: Url) -> EngineFuture<'_, ()>;
    /// Queues a held personal navigation. Call `current` on the UI thread immediately before the
    /// native load, with no second queue hop; false cancels it. Never run inline inside a navigation
    /// hook. The normal navigation policy still applies. Engines lacking this guarantee fail closed.
    ///
    /// # Errors
    /// Missing mount, unsupported dispatch, or failure to queue the operation.
    fn continue_navigation(
        &self,
        id: &MountId,
        url: Url,
        current: Box<dyn FnOnce() -> bool + Send>,
    ) -> Result<(), EngineError> {
        let _ = (id, url, current);
        Err(EngineError::Unsupported(
            "this engine cannot continue a held navigation".to_owned(),
        ))
    }
    /// Back, forward or reload.
    fn history(&self, id: &MountId, step: HistoryStep) -> EngineFuture<'_, ()>;
    /// Shows the webview at `placement`, or hides it (`None`; a hidden webview keeps its last size).
    fn place(&self, id: &MountId, placement: Option<Placement>) -> EngineFuture<'_, ()>;
    /// Temporarily parks the live child for HTML chrome without changing its page or scroll.
    fn browser_overlay(&self, id: &MountId, hidden: bool) -> EngineFuture<'_, ()> {
        let _ = (id, hidden);
        Box::pin(async {
            Err(EngineError::Unsupported(
                "browser overlay is unavailable".to_owned(),
            ))
        })
    }
    /// Gives a HIDDEN webview the size and zoom `placement` without showing it (the device width of a
    /// session that is not on screen changed). Default: hides it (an engine without sizes).
    fn park(&self, id: &MountId, placement: Placement) -> EngineFuture<'_, ()> {
        let _ = placement;
        self.place(id, None)
    }
    /// Closes the webview (its storage stays).
    fn close(&self, id: &MountId) -> EngineFuture<'_, ()>;
    /// Replaces the URL rules of a mounted webview (when a Local mount's ports change) and answers when they
    /// apply. Default: nothing to do (an engine without URL rules).
    fn update_rules(
        &self,
        id: &MountId,
        rules: vw_netguard::rules::RulePage,
    ) -> EngineFuture<'_, ()> {
        let _ = (id, rules);
        Box::pin(async { Ok(()) })
    }
    /// A PNG of what the page shows now.
    fn screenshot(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>>;
    /// A viewport slice for full-page stitching, rendered at CSS size times screen DPR where supported.
    /// Engines without a scaled snapshot keep their ordinary viewport capture as the fallback.
    fn screenshot_slice(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        self.screenshot(id)
    }
    /// A PNG of the whole document (`size`: its CSS width and height) in one render, `size` times the screen's
    /// device pixel ratio large with the page filling it, whatever zoom the preview is shown at (a device width wider
    /// than the panel is shown zoomed out). Default: unsupported (the caller then stitches viewport screenshots).
    fn screenshot_full(&self, id: &MountId, size: (u32, u32)) -> EngineFuture<'_, Vec<u8>> {
        let _ = (id, size);
        Box::pin(async {
            Err(EngineError::Unsupported(
                "this preview engine has no full-page capture".to_owned(),
            ))
        })
    }
    /// Silently prints the mounted document to a paginated PDF, including backgrounds and links.
    /// Native output is staged privately; cancellation must never leave a partial capture file.
    fn print_pdf(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        let _ = id;
        Box::pin(async {
            Err(EngineError::Unsupported(
                "Not available on this system".into(),
            ))
        })
    }
    /// Takes the console entries the page collected since the last drain.
    fn drain_console(&self, id: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>>;
    /// Deletes a session's website data (after its webview closed). Returns what could not be removed yet.
    fn wipe(&self, storage: &Storage) -> EngineFuture<'_, Vec<PathBuf>>;
    /// Inspects fixed session-cookie names in the native store. Never returns values.
    fn accounts(&self, storage: &Storage) -> EngineFuture<'_, Vec<ProviderAccount>> {
        let _ = storage;
        Box::pin(async {
            Err(EngineError::Unsupported(
                "account cookies are unavailable in this engine".to_owned(),
            ))
        })
    }
    /// Deletes provider-domain cookies from the native store.
    fn sign_out(&self, storage: &Storage, provider: Provider) -> EngineFuture<'_, ()> {
        let _ = (storage, provider);
        Box::pin(async {
            Err(EngineError::Unsupported(
                "account cookies are unavailable in this engine".to_owned(),
            ))
        })
    }
    /// The mounted webview's native size in logical pixels (its frame, read from the platform, no pixels taken):
    /// what a layout capture checks for a usable view without trusting page numbers. Default: unknown (`None`).
    fn view_size(&self, id: &MountId) -> EngineFuture<'_, Option<(f64, f64)>> {
        let _ = id;
        Box::pin(async { Ok(None) })
    }
    /// Evaluates `script` natively in the mounted page (no IPC: the page cannot call back) and answers the JSON of
    /// its value. Used for the element/section pick (the element picker). The answer is untrusted page
    /// data. Default: unsupported (an engine that cannot evaluate).
    fn evaluate(&self, id: &MountId, script: String) -> EngineFuture<'_, String> {
        let _ = (id, script);
        Box::pin(async {
            Err(EngineError::Unsupported(
                "this preview engine cannot evaluate page scripts".to_owned(),
            ))
        })
    }
}
