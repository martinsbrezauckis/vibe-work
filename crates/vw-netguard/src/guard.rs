//! [`NetGuard`]: the app-local HTTP proxy the agent browser's webviews send their network traffic through (
//! network guard design). It resolves names itself, refuses targets outside the policy (`decide`) with a 403
//! page naming the reason, and connects to exactly the checked addresses. A canary request
//! (`http://<token>.vw-guard.invalid/`) lets the engine prove that a webview really uses the proxy before any public
//! page is allowed in it.
//!
//! Who is asking: a webview that sends its loopback traffic through the proxy (`WebKitGTK` does) gets
//! a listener of its OWN, per mount ([`NetGuard::proxy_url_for`], keyed by the webview label), so the proxy knows the
//! webview from the port the connection arrived on, never from anything the page sends. A mount keeps one page class
//! for its life (a page from the web, or a page of the session's own server: vw-preview remounts when a navigation
//! crosses classes), so the old page can never use the new page's rights. The core answers per mount whether a
//! loopback port may be reached ([`PageContext`]); Vibe Work's own ports ([`NetGuard::internal_ports`], every
//! listener of the guard) are refused first, whoever asks. The shared listener ([`NetGuard::proxy_url`]) knows no
//! webview: loopback refused.
//!
//! An engine that cannot give each webview its own proxy (`WebView2` needs identical browser arguments for
//! every environment that shares a browser process) uses the app-wide listener ([`NetGuard::proxy_url_all`]). It
//! answers with the union of the live mounts' rights ([`PageContext::mounts`]): a loopback port passes when some
//! mount may reach it, a local-network host when it is some mount's user-opened page. That engine's per-webview
//! request barrier keeps every mount to its own rights; the listener adds the connection to exactly the checked
//! addresses and refuses what no mount may reach.
//!
//! Depends on: tokio, `serve`, `decide`, `resolve`, getrandom. Called by: the app (bind at startup), the core (serve,
//! context, refusal lookups, internal ports), vw-webview (proxy URLs, canary), vw-host (internal ports), tests.
//! Invariants:
//! - Listens on `127.0.0.1` only; at most [`MAX_CONNECTIONS`] connections at once over all listeners (more get 503).
//! - Every refusal is answered before any upstream connection and recorded in a bounded log ([`LOG_CAP`]): host,
//!   port, reason; never paths, queries or bodies.
//! - Every accept loop is a task the guard owns; [`NetGuard::shutdown`] and [`NetGuard::forget_session`] abort them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;

use crate::decide::{Scope, ScopeFuture};
use crate::resolve::Resolve;

/// The reserved host suffix of canary requests (`.invalid` never resolves: only this proxy can answer it).
pub const CANARY_SUFFIX: &str = ".vw-guard.invalid";
/// The path of loopback canary requests (`http://127.0.0.1:<listener port>/__vw_guard/<token>`).
pub const LOOPBACK_CANARY_PATH: &str = "/__vw_guard/";
/// Connections served at once.
pub const MAX_CONNECTIONS: usize = 512;
/// Refusals remembered.
pub const LOG_CAP: usize = 200;
/// Canary tokens kept until collected (bounded).
pub const SEEN_CAP: usize = 256;
/// Hex digits of tokens.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// One refused request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// Increasing number (see [`NetGuard::last_seq`]).
    pub seq: u64,
    /// When.
    pub at: Instant,
    /// The mount (webview label) whose listener got it (`None`: the shared listener).
    pub key: Option<String>,
    /// The host as requested (lowercase, no brackets).
    pub host: String,
    /// The port.
    pub port: u16,
    /// Why (names the address class).
    pub reason: String,
}

/// The core's answer for loopback requests of one webview mount.
pub trait PageContext: Send + Sync {
    /// `Ok` when the webview mounted as `key` (its label) may reach `127.0.0.1:<port>` now (its page class and the
    /// port's owner); else the reason. Vibe Work's own ports never reach this question.
    fn loopback<'a>(&'a self, key: &'a str, port: u16) -> ScopeFuture<'a>;

    /// The host of the local-network page the USER opened in the mount `key` (`None`: not such a page). Default:
    /// none.
    fn lan_host(&self, key: &str) -> Option<String> {
        let _ = key;
        None
    }

    /// The keys (webview labels) of every live mount, for the app-wide listener's union of rights. Default: none
    /// (the app-wide listener then reaches nothing on this computer or the local network).
    fn mounts(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Whose rights a listener applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Who {
    /// No webview: nothing on this computer or the local network.
    Nobody,
    /// One mount (its webview label).
    Mount(String),
    /// Every live mount: the union of their rights (the app-wide listener).
    AnyMount,
}

impl Who {
    /// The mount key refusals are recorded under.
    pub(crate) fn key(&self) -> Option<&str> {
        match self {
            Self::Mount(key) => Some(key),
            Self::Nobody | Self::AnyMount => None,
        }
    }
}

pub(crate) struct Inner {
    pub(crate) resolver: Arc<dyn Resolve>,
    shared: Mutex<Option<std::net::TcpListener>>,
    shared_addr: SocketAddr,
    sessions: Mutex<HashMap<String, (u16, JoinHandle<()>)>>,
    /// The app-wide listener (`proxy_url_all`), bound on first use.
    all: Mutex<Option<(u16, JoinHandle<()>)>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    context: OnceLock<Arc<dyn PageContext>>,
    internal: Mutex<HashSet<u16>>,
    log: Mutex<VecDeque<Refusal>>,
    seq: AtomicU64,
    /// Canary tokens seen and not yet collected, oldest first, at most [`SEEN_CAP`] (a page can send canary-shaped
    /// requests of its own: bounded, oldest dropped).
    seen: Mutex<VecDeque<String>>,
    seen_changed: Notify,
    pub(crate) slots: Arc<Semaphore>,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    /// Records a refusal.
    pub(crate) fn record(&self, session: Option<&str>, host: &str, port: u16, reason: &str) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::info!(host, port, %reason, "browser network guard refused a request");
        let mut log = lock(&self.log);
        log.push_back(Refusal {
            seq,
            at: Instant::now(),
            key: session.map(str::to_owned),
            host: host.to_owned(),
            port,
            reason: reason.to_owned(),
        });
        while log.len() > LOG_CAP {
            log.pop_front();
        }
    }

    /// Marks a canary token as seen (bounded: the oldest is dropped beyond [`SEEN_CAP`]).
    pub(crate) fn saw(&self, token: &str) {
        let mut seen = lock(&self.seen);
        if !seen.iter().any(|t| t == token) {
            seen.push_back(token.to_owned());
            while seen.len() > SEEN_CAP {
                seen.pop_front();
            }
        }
        drop(seen);
        self.seen_changed.notify_waiters();
    }

    /// Takes `token` out of the seen list (true when it was there).
    fn take_seen(&self, token: &str) -> bool {
        let mut seen = lock(&self.seen);
        match seen.iter().position(|t| t == token) {
            Some(at) => {
                seen.remove(at);
                true
            }
            None => false,
        }
    }

    /// True for a port of Vibe Work itself: a registered internal port or a listener of this guard.
    fn is_internal(&self, port: u16) -> bool {
        port == self.shared_addr.port()
            || lock(&self.internal).contains(&port)
            || lock(&self.sessions).values().any(|(p, _)| *p == port)
            || lock(&self.all).as_ref().is_some_and(|(p, _)| *p == port)
    }
}

/// The scope of one listener (whose rights it applies).
pub(crate) struct ListenerScope {
    pub(crate) inner: Arc<Inner>,
    pub(crate) who: Who,
}

/// The most mounts the app-wide listener asks about per request (the core keeps far fewer alive).
const UNION_LIMIT: usize = 64;

impl Scope for ListenerScope {
    fn loopback(&self, port: u16) -> ScopeFuture<'_> {
        Box::pin(async move {
            if self.inner.is_internal(port) {
                return Err(format!(
                    "port {port} belongs to Vibe Work itself and is never opened"
                ));
            }
            let nobody = || "servers on this computer are not reachable from this page".to_owned();
            let Some(context) = self.inner.context.get() else {
                return Err(nobody());
            };
            match &self.who {
                Who::Mount(key) => context.loopback(key, port).await,
                Who::Nobody => Err(nobody()),
                Who::AnyMount => {
                    for key in context.mounts().iter().take(UNION_LIMIT) {
                        if context.loopback(key, port).await.is_ok() {
                            return Ok(());
                        }
                    }
                    Err(format!(
                        "port {port} is not a server any open preview may reach"
                    ))
                }
            }
        })
    }

    fn local_network(&self, host: &str) -> bool {
        let Some(context) = self.inner.context.get() else {
            return false;
        };
        let same = |own: String| own.trim_end_matches('.').eq_ignore_ascii_case(host);
        match &self.who {
            Who::Mount(key) => context.lan_host(key).is_some_and(same),
            Who::Nobody => false,
            Who::AnyMount => context
                .mounts()
                .iter()
                .take(UNION_LIMIT)
                .any(|key| context.lan_host(key).is_some_and(same)),
        }
    }
}

/// The egress guard (cheap to clone).
#[derive(Clone)]
pub struct NetGuard {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for NetGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetGuard")
            .field("shared", &self.inner.shared_addr)
            .finish_non_exhaustive()
    }
}

impl NetGuard {
    /// Binds the shared listener on `127.0.0.1:0` now (no runtime needed); [`Self::serve`] starts it.
    ///
    /// # Errors
    /// The bind error.
    pub fn bind(resolver: Arc<dyn Resolve>) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let shared_addr = listener.local_addr()?;
        Ok(Self {
            inner: Arc::new(Inner {
                resolver,
                shared: Mutex::new(Some(listener)),
                shared_addr,
                sessions: Mutex::new(HashMap::new()),
                all: Mutex::new(None),
                tasks: Mutex::new(Vec::new()),
                context: OnceLock::new(),
                internal: Mutex::new(HashSet::new()),
                log: Mutex::new(VecDeque::new()),
                seq: AtomicU64::new(0),
                seen: Mutex::new(VecDeque::new()),
                seen_changed: Notify::new(),
                slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            }),
        })
    }

    /// The shared proxy URL (`http://127.0.0.1:<port>`): no session, loopback refused.
    #[must_use]
    pub fn proxy_url(&self) -> String {
        format!("http://{}", self.inner.shared_addr)
    }

    /// The shared listener's address.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.inner.shared_addr
    }

    /// Starts the shared listener on the current Tokio runtime (a second call does nothing).
    pub fn serve(&self) {
        let Some(listener) = lock(&self.inner.shared).take() else {
            return;
        };
        match TcpListener::from_std(listener) {
            Ok(listener) => {
                let task = crate::serve::accept_loop(self.inner.clone(), listener, Who::Nobody);
                lock(&self.inner.tasks).push(task);
            }
            Err(error) => tracing::warn!(%error, "the browser network guard could not start"),
        }
    }

    /// The proxy URL of the listener of the webview mount `session` (its label; created on first use; needs a Tokio
    /// runtime).
    ///
    /// # Errors
    /// The bind error.
    pub fn proxy_url_for(&self, session: &str) -> std::io::Result<String> {
        let mut sessions = lock(&self.inner.sessions);
        if let Some((port, _)) = sessions.get(session) {
            return Ok(format!("http://127.0.0.1:{port}"));
        }
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        let listener = TcpListener::from_std(std_listener)?;
        let task =
            crate::serve::accept_loop(self.inner.clone(), listener, Who::Mount(session.to_owned()));
        sessions.insert(session.to_owned(), (port, task));
        Ok(format!("http://127.0.0.1:{port}"))
    }

    /// The proxy URL of the app-wide listener (created on first use, then the same for the app's life;
    /// needs a Tokio runtime): for an engine that sends every webview through one proxy (`WebView2`). It applies
    /// the union of the live mounts' rights ([`PageContext::mounts`]); the engine's per-webview request barrier keeps
    /// each mount to its own.
    ///
    /// # Errors
    /// The bind error.
    pub fn proxy_url_all(&self) -> std::io::Result<String> {
        let mut all = lock(&self.inner.all);
        if let Some((port, _)) = all.as_ref() {
            return Ok(format!("http://127.0.0.1:{port}"));
        }
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        let listener = TcpListener::from_std(std_listener)?;
        let task = crate::serve::accept_loop(self.inner.clone(), listener, Who::AnyMount);
        *all = Some((port, task));
        Ok(format!("http://127.0.0.1:{port}"))
    }

    /// Closes the listener of mount `session` (its webview closed).
    pub fn forget_session(&self, session: &str) {
        if let Some((_, task)) = lock(&self.inner.sessions).remove(session) {
            task.abort();
        }
    }

    /// Stops every listener (quit).
    pub fn shutdown(&self) {
        for task in lock(&self.inner.tasks).drain(..) {
            task.abort();
        }
        for (_, (_, task)) in lock(&self.inner.sessions).drain() {
            task.abort();
        }
        if let Some((_, task)) = lock(&self.inner.all).take() {
            task.abort();
        }
    }

    /// Sets the core's loopback answers (once; later calls are ignored).
    pub fn set_context(&self, context: Arc<dyn PageContext>) {
        let _ = self.inner.context.set(context);
    }

    /// Registers a port of Vibe Work itself (Debug API, key proxy, agent-tools endpoint, dev origin): never reachable
    /// through the guard, whoever asks.
    pub fn add_internal_port(&self, port: u16) {
        lock(&self.inner.internal).insert(port);
    }

    /// Removes a registered internal port (its listener closed).
    pub fn remove_internal_port(&self, port: u16) {
        lock(&self.inner.internal).remove(&port);
    }

    /// Every port of Vibe Work the guard refuses (registered ones and its own listeners).
    #[must_use]
    pub fn internal_ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = lock(&self.inner.internal).iter().copied().collect();
        ports.push(self.inner.shared_addr.port());
        ports.extend(lock(&self.inner.sessions).values().map(|(p, _)| *p));
        ports.extend(lock(&self.inner.all).as_ref().map(|(p, _)| *p));
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// A fresh canary token (32 hex characters).
    #[must_use]
    pub fn new_token() -> String {
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_err() {
            // A predictable token only weakens the proof that the proxy is used; nothing is opened by it.
            bytes = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
                .to_le_bytes();
        }
        bytes
            .iter()
            .flat_map(|b| [HEX[usize::from(b >> 4)], HEX[usize::from(b & 15)]])
            .map(char::from)
            .collect()
    }

    /// The canary URL for `token`.
    #[must_use]
    pub fn canary_url(token: &str) -> String {
        format!("http://{token}{CANARY_SUFFIX}/")
    }

    /// The loopback canary URL for `token` on the listener `proxy_url` (`http://127.0.0.1:<port>`): a navigation to
    /// it reaches the guard in proxy form only when the engine sends loopback traffic through the proxy.
    #[must_use]
    pub fn loopback_canary_url(proxy_url: &str, token: &str) -> String {
        format!(
            "{}{LOOPBACK_CANARY_PATH}{token}",
            proxy_url.trim_end_matches('/')
        )
    }

    /// Waits until the loopback canary of `token` arrived in proxy form, or a CONNECT tunnel to the listener `port`
    /// itself (an engine that tunnels everything), (true) or `limit` passed (false).
    pub async fn wait_seen_loopback(&self, token: &str, port: u16, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;
        let (by_token, by_port) = (format!("loopback:{token}"), format!("loopback-port:{port}"));
        loop {
            let notified = self.inner.seen_changed.notified();
            if self.inner.take_seen(&by_token) || self.inner.take_seen(&by_port) {
                return true;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.inner.take_seen(&by_token) || self.inner.take_seen(&by_port);
            }
        }
    }

    /// Waits until a DIRECT (origin-form) loopback canary request of `token` arrived (true: the engine's request
    /// barrier let a page reach this port) or `limit` passed (false).
    pub async fn wait_seen_direct(&self, token: &str, limit: Duration) -> bool {
        self.wait_seen(&format!("direct:{token}"), limit).await
    }

    /// Waits until the canary request of `token` arrived (true) or `limit` passed (false).
    pub async fn wait_seen(&self, token: &str, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let notified = self.inner.seen_changed.notified();
            if self.inner.take_seen(token) {
                return true;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.inner.take_seen(token);
            }
        }
    }

    /// The newest refusal number (0 before any).
    #[must_use]
    pub fn last_seq(&self) -> u64 {
        self.inner.seq.load(Ordering::SeqCst)
    }

    /// Refusals after `since` (all listeners, oldest first).
    #[must_use]
    pub fn refusals_since(&self, since: u64) -> Vec<Refusal> {
        lock(&self.inner.log)
            .iter()
            .filter(|r| r.seq > since)
            .cloned()
            .collect()
    }
}
