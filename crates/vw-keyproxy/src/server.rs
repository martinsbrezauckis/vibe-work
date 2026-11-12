//! The loopback server: token check, allowlist, key wait, forwarding with the real key, streamed answers.
//!
//! Request order (each step refuses before anything leaves the machine):
//! 1. `Authorization: Bearer <token>` must be the current token → else 401 (`vibe_work_proxy_token`);
//! 2. method + path must be on the allowlist → else 403 (`vibe_work_proxy_path`);
//! 3. a key must be available, waiting up to `ProxyConfig::key_wait` (the owner is told once per
//!    waiting request) → else 503 (`vibe_work_key_unavailable`) with a sentence saying what to do; the connector
//!    catalog and account/config discovery reads never wait: no key → that 503 at once
//!    (held, it outlasted Vibe's 30 s chat start);
//! 4. the request goes upstream with hop-by-hop headers, `Host`, `Cookie` and `Authorization`
//!    removed, `Authorization: Bearer <key>` (marked sensitive) added, the body streamed;
//! 5. the answer's five `x-ratelimit-*` headers (exact names, numbers only, nothing else) are handed to
//!    `ProxyConfig::on_rate_limits` (at most once per second, newest wins: `limits`); then the answer is streamed back
//!    (the connector catalog gains missing tool descriptions); redirects are never followed (a 3xx goes back
//!    unchanged, so the key only goes to the host the proxy chose); an unreachable upstream gives 502
//!    (`vibe_work_upstream_unreachable`).
//!
//! Compaction calls omit image payloads and have a 60 s end-to-end deadline. Other
//! calls bringing a new attachment have a 120 s deadline up to headers, then a 120 s idle limit.
//! Every received chunk renews the idle limit; historical attachments do not bound later calls. Pre-header timeouts
//! and retryable upstream failures return a nonretryable 422; an already started stream ends with
//! a read error on timeout.
//!
//! between steps 2 and 3, a `POST` to the connector gateway is read (at most `connectors::MAX_BODY`) and
//! judged (`connectors.rs`): a forge-writing `tools/call` is answered here and never forwarded. A credential write
//! passes only as a connection without any secret (`credentials.rs`), else 400. A successful connector catalog answer
//! is read (at most `connectors::MAX_CATALOG`) to learn the aliases and the connectable ids, and fill missing tool
//! descriptions.
//!
//! Requests under `/vw-mcp/` skip steps 2-5: `mcp.rs` forwards them to an MCP server with its key.
//!
//! Depends on: axum, reqwest (vw-mistral's client), `key`, `routes`, `token`, `mcp`, `credentials`. Called by: vw-keys.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::key::{KeyCell, KeyNeed};
use crate::limits::{LimitsGate, RateLimits};
use crate::mcp::McpResolverSlot;
use crate::token::TokenSlot;

#[path = "server_forward.rs"]
mod forward;
#[path = "server_resume.rs"]
mod resume;
pub(crate) use forward::error;
use forward::handle;

/// Upstream base URLs (no trailing slash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstreams {
    /// API host.
    pub api: String,
    /// Console host.
    pub console: String,
    /// Chat host.
    pub chat: String,
}

impl Default for Upstreams {
    fn default() -> Self {
        Self {
            api: "https://api.mistral.ai".to_owned(),
            console: "https://console.mistral.ai".to_owned(),
            chat: "https://chat.mistral.ai".to_owned(),
        }
    }
}

/// Called when a request starts waiting for a key (the owner asks the user to unlock or set up).
pub type WaitHook = Arc<dyn Fn(KeyNeed) + Send + Sync>;

/// Called with the rate-limit snapshot of an upstream answer that carried any, at most once per
/// [`crate::LIMITS_EVERY`] (newest wins; a waiting one follows from a short timer task). Runs on the
/// request task or that timer task: keep it cheap and non-blocking.
pub type LimitsHook = Arc<dyn Fn(RateLimits) + Send + Sync>;

/// Proxy settings.
#[derive(Clone)]
pub struct ProxyConfig {
    /// Where requests go.
    pub upstreams: Upstreams,
    /// Longest wait for a key per request. Default 10 minutes (below Vibe's 720 s read timeout,
    /// so Vibe gets our clear 503 instead of its own timeout).
    pub key_wait: Duration,
    /// End-to-end deadline for a Unified Harness context compaction, including request body,
    /// key wait, upstream headers and response body. A failed call releases Vibe's compaction lock.
    pub compaction_deadline: Duration,
    /// Deadline up to headers for new inline images or attachment path notes, then the longest
    /// idle wait between response chunks. Historical attachments do not bound later calls.
    pub image_deadline: Duration,
    /// Told when a request waits for a key.
    pub on_wait: WaitHook,
    /// Told the rate-limit headroom each answer reports.
    pub on_rate_limits: LimitsHook,
    /// The core's MCP binding resolver (`mcp.rs`), filled after the core starts; shared, so a clone of
    /// this config fills the same slot.
    pub mcp: McpResolverSlot,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            upstreams: Upstreams::default(),
            key_wait: Duration::from_secs(600),
            compaction_deadline: Duration::from_secs(60),
            image_deadline: Duration::from_secs(120),
            on_wait: Arc::new(|_| {}),
            on_rate_limits: Arc::new(|_| {}),
            mcp: McpResolverSlot::default(),
        }
    }
}

/// Counters for tests and diagnostics (no request content).
#[derive(Debug, Default)]
pub struct ProxyStats {
    /// Requests forwarded upstream.
    pub forwarded: AtomicU64,
    /// Refused for a wrong or missing token.
    pub bad_token: AtomicU64,
    /// Refused for a path not on the allowlist.
    pub bad_path: AtomicU64,
    /// Answered 503 because no key came in time.
    pub no_key: AtomicU64,
    /// Requests that had to wait for a key.
    pub waited: AtomicU64,
    /// Requests answered immediately because the owner is stopping.
    pub stopping: AtomicU64,
    /// Model requests held for the resumed old turn to end.
    pub resume_waited: AtomicU64,
    /// Model requests refused by a resume barrier or its fresh-turn admission check.
    pub resume_refused: AtomicU64,
}

/// Why the proxy could not start.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// Binding the loopback port failed.
    #[error("the model proxy could not listen on 127.0.0.1: {0}")]
    Bind(std::io::Error),
    /// The HTTPS client could not be built.
    #[error(transparent)]
    Client(#[from] vw_mistral::ClientError),
}

struct Shared {
    config: ProxyConfig,
    /// connector aliases learned from the catalog answers this proxy relayed.
    aliases: crate::connectors::Aliases,
    /// connectors the latest relayed catalog answer marks as needing no sign-in and not connected yet.
    connectable: crate::credentials::Connectable,
    tokens: Arc<TokenSlot>,
    key: KeyCell,
    client: vw_mistral::ClientSlot,
    stats: Arc<ProxyStats>,
    stopping: watch::Receiver<bool>,
    limits: Arc<LimitsGate>,
}

/// A running proxy. Dropping it does not stop the server; call [`ProxyHandle::shutdown`].
pub struct ProxyHandle {
    addr: SocketAddr,
    tokens: Arc<TokenSlot>,
    key: KeyCell,
    stats: Arc<ProxyStats>,
    stopping: watch::Sender<bool>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    limits: Arc<LimitsGate>,
    client: vw_mistral::ClientSlot,
}

impl ProxyHandle {
    /// Starts the proxy on 127.0.0.1 with an OS-chosen port.
    ///
    /// # Errors
    /// [`ProxyError`].
    pub async fn start(
        config: ProxyConfig,
        tokens: Arc<TokenSlot>,
        key: KeyCell,
    ) -> Result<Self, ProxyError> {
        let client = vw_mistral::ClientSlot::new()?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(ProxyError::Bind)?;
        let addr = listener.local_addr().map_err(ProxyError::Bind)?;
        let stats = Arc::new(ProxyStats::default());
        let (stopping, stopping_rx) = watch::channel(false);
        let limits = LimitsGate::new(Arc::clone(&config.on_rate_limits));
        let shared = Arc::new(Shared {
            limits: Arc::clone(&limits),
            config,
            aliases: crate::connectors::Aliases::default(),
            connectable: crate::credentials::Connectable::default(),
            tokens: Arc::clone(&tokens),
            key: key.clone(),
            client: client.clone(),
            stats: Arc::clone(&stats),
            stopping: stopping_rx,
        });
        let app = axum::Router::new().fallback(handle).with_state(shared);
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await;
        });
        Ok(Self {
            addr,
            tokens,
            key,
            stats,
            stopping,
            stop: Some(stop),
            task: Some(task),
            limits,
            client,
        })
    }

    /// The bound address.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port>` (no trailing slash).
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The token slot (the supervisor's secret provider rotates it).
    #[must_use]
    pub fn tokens(&self) -> &Arc<TokenSlot> {
        &self.tokens
    }

    /// The key cell.
    #[must_use]
    pub fn key(&self) -> &KeyCell {
        &self.key
    }

    /// The client that carries the key upstream.
    #[must_use]
    pub fn client(&self) -> &vw_mistral::ClientSlot {
        &self.client
    }

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> &Arc<ProxyStats> {
        &self.stats
    }

    /// Answers requests waiting for a key before the owner closes Vibe's stdin. A 422 tells Vibe
    /// the stopped request must not be retried during its server cleanup.
    pub fn begin_shutdown(&self) {
        use vw_supervisor::ModelRequests;
        self.tokens.requests.shutdown();
        self.stopping.send_replace(true);
    }

    /// Stops accepting requests and waits for the server task (in-flight streams end).
    pub async fn shutdown(mut self) {
        self.begin_shutdown();
        self.tokens.clear();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        }
        self.limits.close();
    }
}
