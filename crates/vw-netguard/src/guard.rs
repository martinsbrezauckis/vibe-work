//! [`NetGuard`]: the app-local HTTP proxy the agent browser's webviews send their network traffic through (
//! network guard design). It resolves names itself, refuses targets outside the policy (`decide`) with a 403
//! page naming the reason, and connects to exactly the checked addresses. A canary request
//! (`http://<token>.vw-guard.invalid/`) lets the engine prove that a webview really uses the proxy before any public
//! page is allowed in it.
//!
//! Who is asking: each listener requires its own random credential before canaries, DNS or forwarding.
//! Its port and credential select the store; neither a page nor another store can choose its rights.
//! A mount keeps one page class for its life (a page from the web, or a page of the
//! session's own server: vw-preview remounts when a navigation crosses classes), so the old page can never use the
//! new page's rights. The core answers per mount whether a loopback port may be reached ([`PageContext`]); Vibe
//! Work's own ports ([`NetGuard::internal_ports`], every listener of the guard) are refused by every scope.
//! A normal shared listener ([`NetGuard::proxy_url`]) knows no webview and refuses loopback.
//! The host-owned [`NetGuard::bind_system_client`] listener separately allows checked local destinations only on
//! native automatic routes; browser and manually configured routes keep their existing scope.
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

use crate::decide::ScopeFuture;
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

/// A service stream was refused before opening, or its selected route failed.
#[cfg(feature = "proxy-transports")]
#[derive(Debug, thiserror::Error)]
pub enum ServiceStreamError {
    /// The guard's checked-address policy refused the target.
    #[error("{0}")]
    Refused(String),
    /// A proxy or direct route failed with the guard's fixed, credential-free reason.
    #[error("{0}")]
    Route(crate::upstream::Error),
}

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
    /// True only for a live browser mount whose requests use normal engine permissions.
    fn browser_addresses(&self, _key: &str) -> bool {
        false
    }

    /// Addresses pinned by a successful proxy connection for one unambiguous live mount.
    /// This is connection evidence, not a frame or document commit.
    fn connected(&self, key: &str, host: &str, port: u16, addresses: &[SocketAddr]) {
        let _ = (key, host, port, addresses);
    }

    /// A final request refusal, after policy and owner checks; never includes URL paths or bodies.
    fn refused(&self, key: Option<&str>, host: &str, port: u16, reason: &str) {
        let _ = (key, host, port, reason);
    }

    /// A final refusal of a request that the listener cannot tell apart (a store with several mounts): `keys` are the
    /// mounts whose page could have sent it. Called next to [`Self::refused`] with no key; never URL paths or bodies.
    fn refused_unattributed(&self, keys: &[String], host: &str, port: u16, reason: &str) {
        let _ = (keys, host, port, reason);
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

    /// `Ok` when a request on a store whose live browser mounts are `keys` may reach the non-public target of
    /// `request` (Local Network Access: the core asks the person for a public page and remembers the answer per site);
    /// else the reason. Vibe Work's own ports never reach this question. Default: refused.
    fn local_access<'a>(
        &'a self,
        keys: &'a [String],
        request: &'a crate::decide::LocalRequest<'a>,
    ) -> ScopeFuture<'a> {
        let _ = (keys, request);
        Box::pin(async {
            Err("this page cannot reach this computer or the local network".to_owned())
        })
    }
}

/// Whose rights a listener applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Who {
    /// No webview. The ordinary shared listener has no local rights; a host-owned automatic relay has its own scope.
    Nobody,
    /// A store and listener generation: an old accepted connection cannot inherit a replacement
    /// listener's members while its accept task is being cancelled.
    Store(String, u64),
}

/// One data store's listener: its port, accept loop and the mounts (webview labels) that use the store now.
struct StoreListener {
    port: u16,
    generation: u64,
    credential: Arc<crate::credential::Credential>,
    task: JoinHandle<()>,
    members: HashSet<String>,
}

pub(crate) struct Inner {
    pub(crate) enabled: tokio::sync::watch::Sender<bool>,
    pub(crate) canaries: std::sync::atomic::AtomicBool,
    pub(crate) resolver: Arc<dyn Resolve>,
    shared: Mutex<Option<std::net::TcpListener>>,
    shared_addr: SocketAddr,
    credential: Arc<crate::credential::Credential>,
    /// The listeners per website data store (`proxy_url_for_store`), by store key.
    stores: Mutex<HashMap<String, StoreListener>>,
    generation: AtomicU64,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    pub(crate) context: OnceLock<Arc<dyn PageContext>>,
    internal: Mutex<HashSet<u16>>,
    log: Mutex<VecDeque<Refusal>>,
    seq: AtomicU64,
    /// Canary tokens seen and not yet collected, oldest first, at most [`SEEN_CAP`] (a page can send canary-shaped
    /// requests of its own: bounded, oldest dropped).
    seen: Mutex<VecDeque<String>>,
    /// Loopback probes announced and still waiting (token, listener port), at most [`SEEN_CAP`]: only these are
    /// answered at a listener's own port ([`NetGuard::announce_loopback`]).
    pub(crate) announced: Mutex<VecDeque<(String, u16)>>,
    seen_changed: Notify,
    pub(crate) slots: Arc<Semaphore>,
    /// The route for public requests and, on the host-owned automatic relay, checked local system routes.
    pub(crate) upstream: std::sync::RwLock<Arc<crate::Upstream>>,
    #[cfg(feature = "proxy-transports")]
    pub(crate) system_client_mode: bool,
    #[cfg(feature = "proxy-transports")]
    system_proxy_fingerprint: Mutex<Option<vw_platform::proxy::ProxyFingerprint>>,
    /// True once this guard gives its own ports to the browser content rules ([`NetGuard::publish_internal_ports`]).
    publishes: std::sync::atomic::AtomicBool,
    /// Held across reading and publishing the ports, so an older snapshot never replaces a newer one.
    publish_lock: Mutex<()>,
    /// This guard's entry in the process-wide listener set ([`crate::listeners`]).
    id: u64,
}

impl Inner {
    /// A fresh native snapshot is required before each host-owned relay connection. A change
    /// revokes older tunnels via the same watch used for vault shutdown; a failed read closes
    /// them and refuses this connection without guessing DIRECT.
    #[cfg(feature = "proxy-transports")]
    pub(crate) fn native_route_ready(&self) -> bool {
        if !self.system_client_mode {
            return true;
        }
        let mut previous = lock(&self.system_proxy_fingerprint);
        observe_system_route(
            &mut previous,
            vw_platform::proxy::fingerprint(),
            &self.enabled,
        )
    }

    /// Puts this guard's current listeners (shared and per store) in the process-wide set. The store lock is held
    /// across the snapshot and the update, so an older snapshot never replaces a newer one (the set takes no guard
    /// lock: the order is always stores, then the set).
    fn sync_listeners(&self) {
        let stores = lock(&self.stores);
        let mut ports: Vec<u16> = stores.values().map(|s| s.port).collect();
        ports.push(self.shared_addr.port());
        crate::listeners::set(self.id, ports);
        drop(stores);
    }
}

#[cfg(feature = "proxy-transports")]
fn observe_system_route<T: Eq>(
    previous: &mut Option<T>,
    next: Result<T, &'static str>,
    enabled: &tokio::sync::watch::Sender<bool>,
) -> bool {
    let Ok(next) = next else {
        let active = *enabled.borrow();
        enabled.send_replace(active);
        return false;
    };
    let changed = previous.as_ref().is_some_and(|old| old != &next);
    *previous = Some(next);
    if changed {
        let active = *enabled.borrow();
        enabled.send_replace(active);
    }
    true
}

impl Drop for Inner {
    fn drop(&mut self) {
        crate::listeners::set(self.id, Vec::new());
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    /// Deliver only to the same sole mount that owned the request before resolution/connection.
    pub(crate) fn connected(
        &self,
        who: &Who,
        key: Option<&str>,
        host: &str,
        port: u16,
        addresses: &[SocketAddr],
    ) {
        if let Some(key) = key
            && self.refusal_key(who).as_deref() == Some(key)
            && let Some(context) = self.context.get()
        {
            context.connected(key, host, port, addresses);
        }
    }

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
            context.refused(session, host, port, reason);
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
    #[cfg(not(feature = "proxy-transports"))]
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

    /// True for a port of Vibe Work itself: a registered internal port, a listener of this guard or of any other
    /// guard in the process.
    fn is_internal(&self, port: u16) -> bool {
        port == self.shared_addr.port()
            || lock(&self.internal).contains(&port)
            || lock(&self.stores).values().any(|s| s.port == port)
            || crate::listeners::contains(port)
    }

    /// The mounts registered on the store `key` now (a snapshot); empty when there are more than [`MEMBER_LIMIT`]
    /// (never a partial list: every member must be asked, so too many grant nothing).
    pub(crate) fn store_members(&self, key: &str, generation: u64) -> Vec<String> {
        lock(&self.stores)
            .get(key)
            .filter(|s| s.generation == generation && s.members.len() <= MEMBER_LIMIT)
            .map_or_else(Vec::new, |s| s.members.iter().cloned().collect())
    }

    /// The mount key a refusal on `who`'s listener is recorded under: the mount, or a store's only member
    /// (several members: none, the request cannot be told apart).
    pub(crate) fn refusal_key(&self, who: &Who) -> Option<String> {
        match who {
            Who::Store(store, generation) => {
                let members = self.store_members(store, *generation);
                match members.as_slice() {
                    [only] => Some(only.clone()),
                    _ => None,
                }
            }
            Who::Nobody => None,
        }
    }
}

/// The most members a store listener asks about per request (the core keeps far fewer alive; more grant nothing).
const MEMBER_LIMIT: usize = 64;

#[path = "guard_scope.rs"]
mod scope;
pub(crate) use scope::ListenerScope;

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
    /// Binds a host-owned relay for core and Vibe native automatic proxy traffic. Only a scheme without a manual
    /// proxy uses the system resolver and may reach checked local addresses. Browser and manual scopes are unchanged.
    ///
    /// # Errors
    /// A bind error or unavailable OS randomness.
    #[cfg(feature = "proxy-transports")]
    pub fn bind_system_client(resolver: Arc<dyn Resolve>) -> std::io::Result<Self> {
        Self::bind_with_mode(resolver, true)
    }

    /// Binds the shared listener on `127.0.0.1:0` now (no runtime needed); [`Self::serve`] starts it.
    ///
    /// # Errors
    /// A bind error or unavailable OS randomness.
    pub fn bind(resolver: Arc<dyn Resolve>) -> std::io::Result<Self> {
        Self::bind_with_mode(resolver, false)
    }

    fn bind_with_mode(
        resolver: Arc<dyn Resolve>,
        system_client_mode: bool,
    ) -> std::io::Result<Self> {
        #[cfg(not(feature = "proxy-transports"))]
        let _ = system_client_mode;
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let shared_addr = listener.local_addr()?;
        let guard = Self {
            inner: Arc::new(Inner {
                enabled: tokio::sync::watch::channel(true).0,
                canaries: std::sync::atomic::AtomicBool::new(true),
                resolver,
                shared: Mutex::new(Some(listener)),
                shared_addr,
                credential: Arc::new(crate::credential::Credential::new()?),
                stores: Mutex::new(HashMap::new()),
                generation: AtomicU64::new(0),
                tasks: Mutex::new(Vec::new()),
                context: OnceLock::new(),
                internal: Mutex::new(HashSet::new()),
                log: Mutex::new(VecDeque::new()),
                seq: AtomicU64::new(0),
                seen: Mutex::new(VecDeque::new()),
                announced: Mutex::new(VecDeque::new()),
                seen_changed: Notify::new(),
                slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
                upstream: std::sync::RwLock::new(Arc::new(crate::Upstream::system())),
                #[cfg(feature = "proxy-transports")]
                system_client_mode,
                #[cfg(feature = "proxy-transports")]
                system_proxy_fingerprint: Mutex::new(None),
                publishes: std::sync::atomic::AtomicBool::new(false),
                publish_lock: Mutex::new(()),
                id: crate::listeners::next_id(),
            }),
        };
        guard.inner.sync_listeners();
        Ok(guard)
    }

    /// Sets the user's proxy for public requests (Settings > Network; the system's proxy is used until this is
    /// called). A host-owned automatic relay also uses it for checked local system routes. Takes effect for the
    /// next connection.
    pub fn set_upstream(&self, upstream: crate::Upstream) {
        *self
            .inner
            .upstream
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(upstream);
    }

    /// Whether this is a host-owned service listener whose automatic routes may use checked local addresses.
    #[cfg(feature = "proxy-transports")]
    #[must_use]
    pub fn is_system_client(&self) -> bool {
        self.inner.system_client_mode
    }

    /// Opens a checked byte stream for a host-owned service request. The caller keeps the original
    /// host for TLS verification; the proxy receives only a checked IP. Disabling this guard closes
    /// the stream, as it does for HTTP tunnels.
    ///
    /// # Errors
    /// A policy refusal or the selected route's fixed error, never a direct retry after a proxy failure.
    #[cfg(feature = "proxy-transports")]
    pub async fn open_service_stream(
        &self,
        request: &url::Url,
    ) -> Result<
        impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
        ServiceStreamError,
    > {
        use crate::decide::decide_for;
        use crate::guard::ServiceStreamError::{Refused, Route};
        use tokio::io::copy_bidirectional;

        if !self.inner.native_route_ready() {
            return Err(Refused("The system proxy route is not ready.".to_owned()));
        }
        if !self.inner.system_client_mode || !*self.inner.enabled.borrow() {
            return Err(Refused("The network connection is unavailable.".to_owned()));
        }
        let secure = match request.scheme() {
            "https" | "wss" => true,
            "http" | "ws" => false,
            _ => return Err(Refused("The address is not valid.".to_owned())),
        };
        let host = request
            .host_str()
            .ok_or_else(|| Refused("The address has no host.".to_owned()))?;
        let port = request
            .port_or_known_default()
            .ok_or_else(|| Refused("The address has no port.".to_owned()))?;
        let mut enabled = self.inner.enabled.subscribe();
        enabled.borrow_and_update();
        let slot = self
            .inner
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Refused("Too many network connections.".to_owned()))?;
        let scope = ListenerScope {
            inner: self.inner.clone(),
            who: Who::Nobody,
            system_client: true,
        };
        let addresses = decide_for(
            self.inner.resolver.as_ref(),
            &scope,
            host,
            port,
            None,
            false,
            secure,
        )
        .await
        .map_err(Refused)?;
        let mut url = request.clone();
        url.set_scheme(if secure { "https" } else { "http" })
            .map_err(|()| Refused("The address is not valid.".to_owned()))?;
        let upstream = self
            .inner
            .upstream
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let (mut target, _) = upstream
            .open_with_system_url(&url, &url, true, &addresses)
            .await
            .map_err(|error| {
                self.inner.record(None, host, port, error.outcome());
                Route(error)
            })?;
        if enabled.has_changed().unwrap_or(true) || !*enabled.borrow() {
            return Err(Refused("The network connection is unavailable.".to_owned()));
        }
        let (client, mut bridge) = tokio::io::duplex(32 * 1024);
        let owner = self.inner.clone();
        tokio::spawn(async move {
            let _owner = owner;
            let _slot = slot;
            tokio::select! {
                biased;
                _ = enabled.changed() => {},
                _ = copy_bidirectional(&mut bridge, &mut target) => {},
            }
        });
        Ok(client)
    }

    /// The most recently completed system route on this listener, for the connection test.
    #[cfg(feature = "proxy-transports")]
    #[must_use]
    pub fn last_route(&self) -> Option<crate::upstream::UsedRoute> {
        self.inner
            .upstream
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .last_route()
    }

    /// Enables forwarding. A change also closes existing connections, including authenticated tunnels.
    /// Used by the vault's proxy forwarder; disabled connections never reach an upstream.
    pub fn set_enabled(&self, enabled: bool) {
        self.inner.enabled.send_replace(enabled);
    }

    /// Turns browser diagnostic responses off for a general proxy forwarder.
    pub fn disable_canaries(&self) {
        self.inner.canaries.store(false, Ordering::SeqCst);
    }

    /// The shared proxy URL with its per-run ingress credential. The ordinary listener has no session or local
    /// rights; a [`Self::bind_system_client`] listener grants checked local access only on automatic routes.
    /// Keep this capability private; it never contains the upstream sign-in.
    #[must_use]
    pub fn proxy_url(&self) -> String {
        self.inner.credential.url(self.inner.shared_addr)
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
                let task = crate::serve::accept_loop(
                    self.inner.clone(),
                    listener,
                    Who::Nobody,
                    self.inner.credential.clone(),
                );
                lock(&self.inner.tasks).push(task);
            }
            Err(error) => tracing::warn!(%error, "the browser network guard could not start"),
        }
    }

    /// The credential-bearing proxy URL of the website data store `store` (keep private), with `label` registered
    /// on it and on no other store (created on first use, then the same port for the store's life; needs a Tokio
    /// runtime): for an engine whose proxy setting belongs to the store (`WKWebsiteDataStore`). It grants only what
    /// every mount registered on the store may reach; [`Self::forget_session`] removes a mount,
    /// [`Self::forget_store`] the listener.
    ///
    /// # Errors
    /// A bind error or unavailable OS randomness.
    pub fn proxy_url_for_store(&self, store: &str, label: &str) -> std::io::Result<String> {
        self.store_url(store, Some(label))
    }

    /// The proxy URL of the listener of the store `store` for a webview that only holds the store and never loads a
    /// page (no member is added: the listener's rights stay those of its members; created on first use, as
    /// [`Self::proxy_url_for_store`]).
    ///
    /// # Errors
    /// A bind error or unavailable OS randomness.
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
            return Ok(listener
                .credential
                .url(([127, 0, 0, 1], listener.port).into()));
        }
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        let listener = TcpListener::from_std(std_listener)?;
        let credential = Arc::new(crate::credential::Credential::new()?);
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst);
        let url = credential.url(([127, 0, 0, 1], port).into());
        let task = crate::serve::accept_loop(
            self.inner.clone(),
            listener,
            Who::Store(store.to_owned(), generation),
            credential.clone(),
        );
        stores.insert(
            store.to_owned(),
            StoreListener {
                port,
                generation,
                credential,
                task,
                members: label.map(str::to_owned).into_iter().collect(),
            },
        );
        drop(stores);
        self.inner.sync_listeners();
        self.republish();
        Ok(url)
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
        let removed = lock(&self.inner.stores).remove(store);
        if let Some(listener) = removed {
            listener.task.abort();
            self.inner.sync_listeners();
            self.republish();
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
        self.inner.sync_listeners();
        self.republish();
    }

    /// Stops every listener (quit).
    pub fn shutdown(&self) {
        lock(&self.inner.shared).take();
        for task in lock(&self.inner.tasks).drain(..) {
            task.abort();
        }
        let mut stores = lock(&self.inner.stores);
        for (_, listener) in stores.drain() {
            listener.task.abort();
        }
        crate::listeners::set(self.inner.id, Vec::new());
        drop(stores);
    }

    /// Sets the core's loopback answers (once; later calls are ignored).
    pub fn set_context(&self, context: Arc<dyn PageContext>) {
        let _ = self.inner.context.set(context);
    }

    /// Registers a port of Vibe Work itself (Debug API, key proxy, agent-tools endpoint, dev origin): never reachable
    /// through the guard, whoever asks.
    pub fn add_internal_port(&self, port: u16) {
        lock(&self.inner.internal).insert(port);
        self.republish();
    }

    /// Removes a registered internal port (its listener closed).
    pub fn remove_internal_port(&self, port: u16) {
        lock(&self.inner.internal).remove(&port);
        self.republish();
    }

    /// Makes this guard's [`Self::internal_ports`] the ports the browser content rules hold for every page
    /// (`rules::set_internal_ports`, macOS: loopback literals never reach the proxy there), now and after every
    /// change. Only the browser's guard does this (one per process); other guards' registered ports stay theirs, while
    /// every guard's listeners are held for every page anyway ([`crate::listeners`]: no guard has to publish them).
    pub fn publish_internal_ports(&self) {
        self.inner.publishes.store(true, Ordering::SeqCst);
        self.republish();
    }

    fn republish(&self) {
        if self.inner.publishes.load(Ordering::SeqCst) {
            let _one_at_a_time = lock(&self.inner.publish_lock);
            crate::rules::set_internal_ports(self.own_ports());
        }
    }

    /// Vibe Work's own ports for strict backend scopes (registered ones, this guard's listeners and every other
    /// guard's listeners in the process).
    #[must_use]
    pub fn internal_ports(&self) -> Vec<u16> {
        let mut ports = self.own_ports();
        ports.extend(crate::listeners::ports());
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// The ports this guard publishes: registered ones and its own listeners. Other guards' listeners are not copied
    /// here (the rules read them live from [`crate::listeners`], so a closed guard's ports never linger).
    fn own_ports(&self) -> Vec<u16> {
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
            url::Url::parse(proxy_url).map_or_else(
                |_| "http://127.0.0.1:0".to_owned(),
                |url| url.origin().ascii_serialization()
            )
        )
    }

    /// Announces the loopback canary of `token` on the listener `port` before it is loaded: a listener answers a
    /// canary at its own port only for an announced probe, until [`Self::wait_seen_loopback`] for it returns; any
    /// other request for that port is refused like every page's request for Vibe Work's own ports.
    pub fn announce_loopback(&self, token: &str, port: u16) {
        let mut announced = lock(&self.inner.announced);
        announced.push_back((token.to_owned(), port));
        while announced.len() > SEEN_CAP {
            announced.pop_front();
        }
    }

    /// Waits until the announced loopback canary of `token` arrived in proxy form, or a CONNECT tunnel to the
    /// listener `port` itself (an engine that tunnels everything), (true) or `limit` passed (false); then ends the
    /// announcement.
    pub async fn wait_seen_loopback(&self, token: &str, port: u16, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;
        let (by_token, by_port) = (format!("loopback:{token}"), format!("loopback-port:{port}"));
        let seen = loop {
            let notified = self.inner.seen_changed.notified();
            if self.inner.take_seen(&by_token) || self.inner.take_seen(&by_port) {
                break true;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                break self.inner.take_seen(&by_token) || self.inner.take_seen(&by_port);
            }
        };
        lock(&self.inner.announced).retain(|(t, p)| !(t == token && *p == port));
        seen
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

    /// Whether a refused request failed while reading the system proxy configuration.
    /// The fixed refusal reason is recorded before the guard answers 502; no destination or PAC text is returned.
    #[cfg(feature = "proxy-transports")]
    #[must_use]
    pub fn proxy_lookup_failed_since(&self, since: u64) -> bool {
        lock(&self.inner.log).iter().any(|refusal| {
            refusal.seq > since && refusal.reason == crate::upstream::Error::Discovery.outcome()
        })
    }

    /// Whether a selected upstream proxy could not be contacted before it answered.
    /// Only the fixed reason in this guard's refusal log is read; direct route failures are excluded.
    #[cfg(feature = "proxy-transports")]
    #[must_use]
    pub fn proxy_connect_failed_since(&self, since: u64) -> bool {
        lock(&self.inner.log).iter().any(|refusal| {
            refusal.seq > since
                && refusal.reason == crate::upstream::Error::ProxyUnreachable.outcome()
        })
    }
}

#[cfg(all(test, feature = "proxy-transports"))]
mod native_route_tests {
    #[test]
    fn changed_or_unreadable_system_proxy_revokes_old_connections_without_unlocking() {
        let (enabled, mut existing) = tokio::sync::watch::channel(true);
        let mut previous = None;
        existing.borrow_and_update();
        assert!(super::observe_system_route(
            &mut previous,
            Ok(1_u8),
            &enabled
        ));
        assert!(!existing.has_changed().unwrap());
        assert!(super::observe_system_route(
            &mut previous,
            Ok(1_u8),
            &enabled
        ));
        assert!(!existing.has_changed().unwrap());
        assert!(super::observe_system_route(
            &mut previous,
            Ok(2_u8),
            &enabled
        ));
        assert!(existing.has_changed().unwrap());
        assert!(*existing.borrow_and_update());

        enabled.send_replace(false);
        existing.borrow_and_update();
        assert!(super::observe_system_route(
            &mut previous,
            Ok(3_u8),
            &enabled
        ));
        assert!(existing.has_changed().unwrap());
        assert!(!*existing.borrow_and_update());
        assert!(!super::observe_system_route::<u8>(
            &mut previous,
            Err("unavailable"),
            &enabled
        ));
        assert!(existing.has_changed().unwrap());
        assert!(!*existing.borrow_and_update());
    }
}
