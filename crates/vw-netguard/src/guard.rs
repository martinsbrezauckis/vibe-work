//! [`NetGuard`]: the app-local HTTP proxy the agent browser's webviews send their network traffic through (
//! network guard design). It resolves names itself, refuses targets outside the policy (`decide`) with a 403
//! page naming the reason, and connects to exactly the checked addresses. A canary request
//! (`http://<token>.vw-guard.invalid/`) lets the engine prove that a webview really uses the proxy before any public
//! page is allowed in it.
//!
//! Who is asking: the proxy knows the webviews behind a connection from the listener (port) it arrived on, never from
//! anything the page sends. A mount keeps one page class for its life (a page from the web, or a page of the
//! session's own server: vw-preview remounts when a navigation crosses classes), so the old page can never use the
//! new page's rights. The core answers per mount whether a loopback port may be reached ([`PageContext`]); Vibe
//! Work's own ports ([`NetGuard::internal_ports`], every listener of the guard) are refused first, whoever asks. The
//! shared listener ([`NetGuard::proxy_url`]) knows no webview: loopback refused.
//!
//! Every engine keeps the proxy setting on something several webviews share, so the guard has a listener per STORE
//! ([`NetGuard::proxy_url_for_store`]): the website data store on macOS 14+ (`WKWebsiteDataStore`
//! `proxyConfigurations`), the user data folder on Windows (`WebView2` takes the proxy from the browser arguments of
//! the folder's one browser process, which every environment on that folder must share), the data folder on Linux
//! (`WebKitGTK` keeps the proxy on the folder's web context). One store can serve
//! several mounts at once (the shared public-web store serves every session's web pages), and a store's proxy cannot
//! change while it is in use (`WebKit` bug 316948: pooled connections; `WebView2`: the browser process keeps its
//! arguments), so the store keeps one port for its life. It grants only what EVERY mount registered on that store
//! may reach (a store shared by a session's own-server page and the local-network page
//! the user opened must not let either use the other's rights; the shared web store's members are web pages:
//! nothing on this computer or the local network; no members: nothing). A mount is a member of one store at a time.
//! Each engine keeps every mount to its own rights for what passes around or unread through the proxy where it can
//! (macOS: URL content rules for address literals, which `WebKit` sends around the proxy; Windows: the per-webview
//! request barrier for HTTP; Linux: everything goes through the proxy).
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
    /// A final request refusal, after policy and owner checks; never includes URL paths or bodies.
    fn refused(&self, key: Option<&str>, reason: &str) {
        let _ = (key, reason);
    }

    /// `Ok` when the webview mounted as `key` (its label) may reach `127.0.0.1:<port>` now (its page class and the
    /// port's owner); else the reason. Vibe Work's own ports never reach this question.
    fn loopback<'a>(&'a self, key: &'a str, port: u16) -> ScopeFuture<'a>;

    /// The host of the local-network page the USER opened in the mount `key` (`None`: not such a page). Default:
    /// none.
    fn lan_host(&self, key: &str) -> Option<String> {
        let _ = key;
        None
    }
}

/// Whose rights a listener applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Who {
    /// No webview: nothing on this computer or the local network.
    Nobody,
    /// The mounts registered on one website data store (its key): only what every one of them may reach.
    Store(String),
}

/// One data store's listener: its port, accept loop and the mounts (webview labels) that use the store now.
struct StoreListener {
    port: u16,
    task: JoinHandle<()>,
    members: HashSet<String>,
}

pub(crate) struct Inner {
    pub(crate) resolver: Arc<dyn Resolve>,
    shared: Mutex<Option<std::net::TcpListener>>,
    shared_addr: SocketAddr,
    /// The listeners per website data store (`proxy_url_for_store`), by store key.
    stores: Mutex<HashMap<String, StoreListener>>,
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
    /// The user's proxy for public requests ([`NetGuard::set_upstream`]; the system's until then).
    pub(crate) upstream: std::sync::RwLock<Arc<crate::Upstream>>,
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
        drop(log);
        if let Some(context) = self.context.get() {
            context.refused(session, reason);
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

    /// The user's proxy hop for a public `host:port` (`tls`: a tunnel), `None` for a direct connection.
    pub(crate) fn upstream_route(
        &self,
        tls: bool,
        host: &str,
        port: u16,
    ) -> Option<crate::upstream::Route> {
        self.upstream
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .route(tls, host, port)
    }

    /// True for a port of Vibe Work itself: a registered internal port or a listener of this guard.
    fn is_internal(&self, port: u16) -> bool {
        port == self.shared_addr.port()
            || lock(&self.internal).contains(&port)
            || lock(&self.stores).values().any(|s| s.port == port)
    }

    /// The mounts registered on the store `key` now (a snapshot); empty when there are more than [`MEMBER_LIMIT`]
    /// (never a partial list: every member must be asked, so too many grant nothing).
    fn store_members(&self, key: &str) -> Vec<String> {
        lock(&self.stores)
            .get(key)
            .filter(|s| s.members.len() <= MEMBER_LIMIT)
            .map_or_else(Vec::new, |s| s.members.iter().cloned().collect())
    }

    /// The mount key a refusal on `who`'s listener is recorded under: the mount, or a store's only member
    /// (several members: none, the request cannot be told apart).
    pub(crate) fn refusal_key(&self, who: &Who) -> Option<String> {
        match who {
            Who::Store(store) => {
                let members = self.store_members(store);
                match members.as_slice() {
                    [only] => Some(only.clone()),
                    _ => None,
                }
            }
            Who::Nobody => None,
        }
    }
}

/// The scope of one listener (whose rights it applies).
pub(crate) struct ListenerScope {
    pub(crate) inner: Arc<Inner>,
    pub(crate) who: Who,
}

/// The most members a store listener asks about per request (the core keeps far fewer alive; more grant nothing).
const MEMBER_LIMIT: usize = 64;

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
                Who::Nobody => Err(nobody()),
                Who::Store(store) => {
                    // Every member must have the right (never one mount's right used by another).
                    let members = self.inner.store_members(store);
                    if members.is_empty() {
                        return Err(nobody());
                    }
                    for key in &members {
                        context.loopback(key, port).await?;
                    }
                    Ok(())
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
            Who::Nobody => false,
            Who::Store(store) => {
                let members = self.inner.store_members(store);
                !members.is_empty()
                    && members
                        .iter()
                        .all(|key| context.lan_host(key).is_some_and(same))
            }
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
                stores: Mutex::new(HashMap::new()),
                tasks: Mutex::new(Vec::new()),
                context: OnceLock::new(),
                internal: Mutex::new(HashSet::new()),
                log: Mutex::new(VecDeque::new()),
                seq: AtomicU64::new(0),
                seen: Mutex::new(VecDeque::new()),
                seen_changed: Notify::new(),
                slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
                upstream: std::sync::RwLock::new(Arc::new(crate::Upstream::system())),
            }),
        })
    }

    /// Sets the user's proxy for public requests (Settings > Network; the system's proxy is used until this is
    /// called). Takes effect for the next connection.
    pub fn set_upstream(&self, upstream: crate::Upstream) {
        *self
            .inner
            .upstream
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(upstream);
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

    /// The proxy URL of the listener of the website data store `store` (its key), with the mount `label` registered
    /// on it and on no other store (created on first use, then the same port for the store's life; needs a Tokio
    /// runtime): for an engine whose proxy setting belongs to the store (`WKWebsiteDataStore`). It grants only what
    /// every mount registered on the store may reach; [`Self::forget_session`] removes a mount,
    /// [`Self::forget_store`] the listener.
    ///
    /// # Errors
    /// The bind error.
    pub fn proxy_url_for_store(&self, store: &str, label: &str) -> std::io::Result<String> {
        self.store_url(store, Some(label))
    }

    /// The proxy URL of the listener of the store `store` for a webview that only holds the store and never loads a
    /// page (no member is added: the listener's rights stay those of its members; created on first use, as
    /// [`Self::proxy_url_for_store`]).
    ///
    /// # Errors
    /// The bind error.
    pub fn store_proxy_url(&self, store: &str) -> std::io::Result<String> {
        self.store_url(store, None)
    }

    fn store_url(&self, store: &str, label: Option<&str>) -> std::io::Result<String> {
        let mut stores = lock(&self.inner.stores);
        // One store per mount: a label left on another store (a mount that moved) never keeps rights there.
        if let Some(label) = label {
            for (key, listener) in stores.iter_mut() {
                if key != store {
                    listener.members.remove(label);
                }
            }
        }
        if let Some(listener) = stores.get_mut(store) {
            if let Some(label) = label {
                listener.members.insert(label.to_owned());
            }
            return Ok(format!("http://127.0.0.1:{}", listener.port));
        }
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        let listener = TcpListener::from_std(std_listener)?;
        let task =
            crate::serve::accept_loop(self.inner.clone(), listener, Who::Store(store.to_owned()));
        stores.insert(
            store.to_owned(),
            StoreListener {
                port,
                task,
                members: label.map(str::to_owned).into_iter().collect(),
            },
        );
        Ok(format!("http://127.0.0.1:{port}"))
    }

    /// Removes the mount `session` (its webview label; the webview closed) from its data store's listener (that
    /// listener stays: the store keeps its proxy setting).
    pub fn forget_session(&self, session: &str) {
        for listener in lock(&self.inner.stores).values_mut() {
            listener.members.remove(session);
        }
    }

    /// Closes the listener of the website data store `store` (the store was deleted; a later store with the same key
    /// starts fresh, on a new port).
    pub fn forget_store(&self, store: &str) {
        if let Some(listener) = lock(&self.inner.stores).remove(store) {
            listener.task.abort();
        }
    }

    /// Closes the listeners of every store whose key starts with `prefix` (a folder and the folders inside it were
    /// deleted; see [`Self::forget_store`]).
    pub fn forget_stores_under(&self, prefix: &str) {
        lock(&self.inner.stores).retain(|key, listener| {
            let keep = !key.starts_with(prefix);
            if !keep {
                listener.task.abort();
            }
            keep
        });
    }

    /// Stops every listener (quit).
    pub fn shutdown(&self) {
        for task in lock(&self.inner.tasks).drain(..) {
            task.abort();
        }
        for (_, listener) in lock(&self.inner.stores).drain() {
            listener.task.abort();
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
        ports.extend(lock(&self.inner.stores).values().map(|s| s.port));
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
