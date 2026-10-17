//! The loopback server: token check, allowlist, key wait, forwarding with the real key, streamed
//! answers.
//!
//! Request order (each step refuses before anything leaves the machine):
//! 1. `Authorization: Bearer <token>` must be the current token → else 401 (`vibe_work_proxy_token`);
//! 2. method + path must be on the allowlist → else 403 (`vibe_work_proxy_path`);
//! 3. a key must be available, waiting up to `ProxyConfig::key_wait` (the owner is told once per
//!    waiting request) → else 503 (`vibe_work_key_unavailable`) with a sentence saying what to do; the connector
//!    catalog (`GET /v1/connectors/bootstrap`, fetched when a chat starts) never waits: no key → that 503 at once
//!    (held, it outlasted Vibe's 30 s chat start);
//! 4. the request goes upstream with hop-by-hop headers, `Host`, `Cookie` and `Authorization`
//!    removed, `Authorization: Bearer <key>` (marked sensitive) added, the body streamed;
//! 5. the answer's five `x-ratelimit-*` headers (exact names, numbers only, nothing else) are
//!    handed to `ProxyConfig::on_rate_limits`; then the answer is streamed back (the connector catalog gains missing
//!    tool descriptions); redirects are never
//!    followed (a 3xx goes back unchanged, so the key only goes to the host the proxy chose); an
//!    upstream that cannot be reached gives 502 (`vibe_work_upstream_unreachable`).
//!
//! Compaction calls omit image payloads and have a 60 s end-to-end deadline. Other
//! attachment-bearing calls have a 120 s deadline, including streamed bytes. Pre-header timeouts
//! and retryable upstream failures return a nonretryable 422; an already started stream ends with
//! a read error on timeout.
//!
//! between steps 2 and 3, a `POST` to the connector gateway is read (at most `connectors::MAX_BODY`) and
//! judged (`connectors.rs`): a forge-writing `tools/call` is answered here and never forwarded. A successful connector
//! catalog answer is read (at most `connectors::MAX_CATALOG`) to learn the aliases and fill missing tool descriptions.
//!
//! Requests under `/vw-mcp/` skip steps 2-5: `mcp.rs` forwards them to an MCP server with its key.
//!
//! Depends on: axum, reqwest (vw-mistral's client), `key`, `routes`, `token`, `mcp`. Called by: vw-keys.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{ACCEPT_ENCODING, AUTHORIZATION, CONTENT_LENGTH, HeaderName, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{StreamExt, TryStreamExt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::compaction::{MAX_CHAT_BODY, prepare};
use crate::key::{KeyCell, KeyNeed, KeyState};
use crate::limits::{RateLimits, now_ms};
use crate::mcp::{MCP_PREFIX, McpResolverSlot};
use crate::routes::{RouteSpec, Upstream, match_route};
use crate::token::TokenSlot;

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

/// Called with the rate-limit snapshot of an upstream answer that carried any. Runs on the
/// request task before the answer is passed on: keep it cheap and non-blocking.
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
    /// End-to-end deadline for a chat request carrying inline images or model-facing attachment
    /// path notes, including streamed response bytes. Ordinary text-only streams are unchanged.
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
    tokens: Arc<TokenSlot>,
    key: KeyCell,
    client: reqwest::Client,
    stats: Arc<ProxyStats>,
}

/// A running proxy. Dropping it does not stop the server; call [`ProxyHandle::shutdown`].
pub struct ProxyHandle {
    addr: SocketAddr,
    tokens: Arc<TokenSlot>,
    key: KeyCell,
    stats: Arc<ProxyStats>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
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
        let client = vw_mistral::proxy_client()?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(ProxyError::Bind)?;
        let addr = listener.local_addr().map_err(ProxyError::Bind)?;
        let stats = Arc::new(ProxyStats::default());
        let shared = Arc::new(Shared {
            config,
            aliases: crate::connectors::Aliases::default(),
            tokens: Arc::clone(&tokens),
            key: key.clone(),
            client,
            stats: Arc::clone(&stats),
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
            stop: Some(stop),
            task: Some(task),
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

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> &Arc<ProxyStats> {
        &self.stats
    }

    /// Stops accepting requests and waits for the server task (in-flight streams end).
    pub async fn shutdown(mut self) {
        self.tokens.clear();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        }
    }
}

fn error(status: StatusCode, kind: &str, message: &str) -> Response {
    let body = serde_json::json!({"error": {"type": kind, "message": message}, "object": "error"});
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// The 422 keeps Vibe's retry policy from repeating a slow compaction for several minutes.
fn compaction_timeout() -> Response {
    error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "vibe_work_compaction_timeout",
        "Context compaction took too long. The turn was stopped; try your next message or compact again.",
    )
}

fn attachment_timeout() -> Response {
    error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "vibe_work_attachment_timeout",
        "The attachment request took too long. The turn was stopped; try your next message.",
    )
}

/// A refusal decided before anything is forwarded (small, so `Result`s stay cheap).
struct Refusal {
    status: StatusCode,
    kind: &'static str,
    message: &'static str,
}

fn refuse(status: StatusCode, kind: &'static str, message: &'static str) -> Refusal {
    Refusal {
        status,
        kind,
        message,
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// Headers never copied in either direction (RFC 9110 §7.6.1 hop-by-hop, plus our own rules).
fn dropped(name: &HeaderName, request: bool) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    ) || (request
        && matches!(
            name.as_str(),
            "host" | "authorization" | "cookie" | "x-api-key"
        ))
}

/// Steps 1 and 2: the current token and an allowlisted endpoint.
fn admit(
    shared: &Shared,
    parts: &axum::http::request::Parts,
) -> Result<&'static RouteSpec, Refusal> {
    if !bearer(&parts.headers).is_some_and(|t| shared.tokens.accepts(t)) {
        shared.stats.bad_token.fetch_add(1, Ordering::Relaxed);
        warn!(method = %parts.method, "model proxy: refused a request without the current token");
        return Err(refuse(
            StatusCode::UNAUTHORIZED,
            "vibe_work_proxy_token",
            "Vibe Work's model proxy refused this request: it does not carry the current proxy token.",
        ));
    }
    let path = parts.uri.path();
    match_route(&parts.method, path).ok_or_else(|| {
        shared.stats.bad_path.fetch_add(1, Ordering::Relaxed);
        warn!(method = %parts.method, path, "model proxy: refused a path not on the allowlist");
        refuse(
            StatusCode::FORBIDDEN,
            "vibe_work_proxy_path",
            "Vibe Work's model proxy does not forward this Mistral endpoint.",
        )
    })
}

/// The sentence a 503 `vibe_work_key_unavailable` carries: what the owner does to get a key in place.
fn key_unavailable(need: KeyNeed) -> &'static str {
    match need {
        KeyNeed::Unlock => {
            "Vibe Work's vault is locked. Unlock it in Vibe Work, then run the turn again."
        }
        KeyNeed::Setup => {
            "No Mistral API key is set up in Vibe Work. Add one in Settings, then run the turn again."
        }
    }
}

/// Step 3: the key as a sensitive `Authorization` value, waiting for it when needed.
///
/// `wait` false (the connector catalog): one snapshot of the key decides, never a wait and never the owner's
/// hook. Vibe fetches the catalog when a chat starts and gives it 30 s; held for a key, it outlasted the chat start (a
/// new chat with the vault locked or no key timed out and stuck). The 503 instead makes Vibe start the chat without
/// connectors, and it caches only a successful catalog, so the next chat after an unlock loads them. One snapshot, not
/// a check before a wait: a vault that locks between the two can no longer send the catalog into the wait.
async fn authorization(shared: &Shared, wait: bool) -> Result<HeaderValue, Refusal> {
    let unavailable = |need: KeyNeed| {
        shared.stats.no_key.fetch_add(1, Ordering::Relaxed);
        refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "vibe_work_key_unavailable",
            key_unavailable(need),
        )
    };
    let key = if wait {
        let stats = Arc::clone(&shared.stats);
        let hook = Arc::clone(&shared.config.on_wait);
        shared
            .key
            .wait_ready(shared.config.key_wait, move |need| {
                stats.waited.fetch_add(1, Ordering::Relaxed);
                hook(need);
            })
            .await
            .map_err(unavailable)?
    } else {
        match shared.key.current() {
            KeyState::Ready(key) => key,
            KeyState::Missing(need) => return Err(unavailable(need)),
        }
    };
    let value = zeroize::Zeroizing::new(format!("Bearer {}", key.as_str()));
    let mut auth = HeaderValue::from_str(&value).map_err(|_| {
        refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "vibe_work_key_unavailable",
            "The stored Mistral key contains characters a key cannot have. Replace it in Settings.",
        )
    })?;
    auth.set_sensitive(true);
    Ok(auth)
}

/// The body to send upstream: streamed unread for every route but a `POST` to the connector gateway, which is read
/// and judged first (`connectors.rs`). `Err` = the answer to give instead (never forwarded).
async fn gateway_body(
    shared: &Shared,
    route: &RouteSpec,
    parts: &axum::http::request::Parts,
    body: Body,
) -> Result<reqwest::Body, Box<Response>> {
    if route.template != crate::connectors::GATEWAY_TEMPLATE
        || parts.method != axum::http::Method::POST
    {
        return Ok(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    let Ok(bytes) = axum::body::to_bytes(body, crate::connectors::MAX_BODY).await else {
        return Err(Box::new(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "vibe_work_connector_request",
            "Vibe Work's model proxy refused a connector request that is too large to check.",
        )));
    };
    // `/v1/connectors-gateway/<id>/mcp`: the id is the fourth segment (the allowlist checked its shape).
    let id = parts.uri.path().split('/').nth(3).unwrap_or("");
    match crate::connectors::judge(&bytes, id, &shared.aliases) {
        crate::connectors::Verdict::Forward => Ok(reqwest::Body::from(bytes)),
        crate::connectors::Verdict::Answer(value) => Err(Box::new(
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                value.to_string(),
            )
                .into_response(),
        )),
        crate::connectors::Verdict::Refuse(reason) => Err(Box::new(error(
            StatusCode::BAD_REQUEST,
            "vibe_work_connector_request",
            reason,
        ))),
    }
}

/// Learns connector aliases and fills missing tool descriptions in a successful catalog answer.
/// An answer over `connectors::MAX_CATALOG` teaches nothing and passes through unchanged.
async fn relay_catalog(
    shared: &Shared,
    mut out: axum::http::response::Builder,
    response: reqwest::Response,
) -> Response {
    let unreachable = || {
        error(
            StatusCode::BAD_GATEWAY,
            "vibe_work_upstream_unreachable",
            "Mistral's answer could not be passed on.",
        )
    };
    let Ok(body) = response.bytes().await else {
        return unreachable();
    };
    if body.len() > crate::connectors::MAX_CATALOG {
        return out.body(Body::from(body)).unwrap_or_else(|_| unreachable());
    }
    shared.aliases.learn(&body);
    if let Some(described) = crate::connectors::describe_catalog(&body) {
        if let Some(headers) = out.headers_mut() {
            headers.remove(CONTENT_LENGTH);
            if let Ok(length) = HeaderValue::from_str(&described.len().to_string()) {
                headers.insert(CONTENT_LENGTH, length);
            }
        }
        return out
            .body(Body::from(described))
            .unwrap_or_else(|_| unreachable());
    }
    out.body(Body::from(body)).unwrap_or_else(|_| unreachable())
}

struct PreparedBody {
    body: reqwest::Body,
    deadline: Option<tokio::time::Instant>,
    compaction: bool,
    rewritten_len: Option<usize>,
}

fn timed_out(compaction: bool) -> Response {
    if compaction {
        compaction_timeout()
    } else {
        attachment_timeout()
    }
}

/// Read just enough to identify a Vibe compaction. Other routes keep their existing body rule.
async fn prepared_body(
    shared: &Shared,
    route: &RouteSpec,
    parts: &axum::http::request::Parts,
    body: Body,
    arrived: tokio::time::Instant,
) -> Result<PreparedBody, Box<Response>> {
    // Vibe's compaction purpose is in JSON; bound the read before inspecting it. Agent calls
    // remain byte-for-byte unchanged after this step.
    if route.template == "/v1/chat/completions" {
        if parts
            .headers
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|n| n > MAX_CHAT_BODY)
        {
            return Err(Box::new(error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "vibe_work_chat_too_large",
                "The model request is too large; remove some attachments or start a new chat.",
            )));
        }
        let read = tokio::time::timeout_at(
            arrived + shared.config.compaction_deadline,
            axum::body::to_bytes(body, MAX_CHAT_BODY),
        )
        .await;
        let bytes = match read {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => {
                return Err(Box::new(error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "vibe_work_chat_too_large",
                    "The model request is too large; remove some attachments or start a new chat.",
                )));
            }
            Err(_) => {
                return Err(Box::new(error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "vibe_work_chat_upload_timeout",
                    "The model request upload took too long; try again with fewer attachments.",
                )));
            }
        };
        let (rewritten, shape) = prepare(&bytes);
        debug!(
            compaction = shape.compaction,
            images = shape.images,
            attachment_notes = shape.attachment_notes,
            image_url_bytes = shape.image_url_bytes,
            omitted = shape.omitted,
            body_bytes = bytes.len(),
            "model proxy: chat request shape"
        );
        let deadline = if shape.compaction {
            Some(arrived + shared.config.compaction_deadline)
        } else if shape.images > 0 || shape.attachment_notes > 0 {
            Some(arrived + shared.config.image_deadline)
        } else {
            None
        };
        let rewritten_len = rewritten.as_ref().map(Vec::len);
        let body = rewritten.map_or_else(|| reqwest::Body::from(bytes), reqwest::Body::from);
        Ok(PreparedBody {
            body,
            deadline,
            compaction: shape.compaction,
            rewritten_len,
        })
    } else {
        let body = gateway_body(shared, route, parts, body).await?;
        Ok(PreparedBody {
            body,
            deadline: None,
            compaction: false,
            rewritten_len: None,
        })
    }
}

async fn handle(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    let arrived = tokio::time::Instant::now();
    let (parts, body) = request.into_parts();
    // MCP servers use their own route and credentials.
    if parts.uri.path().starts_with(MCP_PREFIX) {
        let tokens = Arc::clone(&shared.tokens);
        let accepts = move |t: &str| tokens.accepts(t);
        return crate::mcp::handle(&shared.client, &shared.config.mcp, &accepts, parts, body).await;
    }
    let route = match admit(&shared, &parts) {
        Ok(route) => route,
        Err(r) => return error(r.status, r.kind, r.message),
    };
    // The connector catalog never waits for a key (`authorization`).
    let wait = route.template != crate::connectors::BOOTSTRAP_TEMPLATE;
    // connector calls are checked before the key is used.
    let PreparedBody {
        body,
        deadline,
        compaction,
        rewritten_len,
    } = match prepared_body(&shared, route, &parts, body, arrived).await {
        Ok(prepared) => prepared,
        Err(answer) => return *answer,
    };
    let auth = match deadline {
        Some(deadline) => {
            match tokio::time::timeout_at(deadline, authorization(&shared, wait)).await {
                Ok(Ok(auth)) => auth,
                Ok(Err(r)) => {
                    return error(StatusCode::UNPROCESSABLE_ENTITY, r.kind, r.message);
                }
                Err(_) => return timed_out(compaction),
            }
        }
        None => match authorization(&shared, wait).await {
            Ok(auth) => auth,
            Err(r) => return error(r.status, r.kind, r.message),
        },
    };
    let base = match route.upstream {
        Upstream::Api => &shared.config.upstreams.api,
        Upstream::Console => &shared.config.upstreams.console,
        Upstream::Chat => &shared.config.upstreams.chat,
    };
    let path = parts.uri.path();
    let rest = path.strip_prefix(route.strip).unwrap_or(path);
    let query = parts
        .uri
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in &parts.headers {
        if !dropped(name, true) {
            headers.append(name.clone(), value.clone());
        }
    }
    if let Some(len) = rewritten_len {
        headers.insert(
            CONTENT_LENGTH,
            HeaderValue::from_str(&len.to_string()).unwrap_or_else(|_| unreachable!()),
        );
    }
    // The proxy's streaming client does not decode compressed bodies. Vibe's HTTP client advertises
    // compression, so request plain JSON for the one response we must inspect and rewrite.
    if route.template == crate::connectors::BOOTSTRAP_TEMPLATE {
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    }
    headers.insert(AUTHORIZATION, auth);
    let sending = shared
        .client
        .request(parts.method.clone(), format!("{base}{rest}{query}"))
        .headers(headers)
        .body(body)
        .send();
    let sent = match deadline {
        Some(deadline) => match tokio::time::timeout_at(deadline, sending).await {
            Ok(sent) => sent,
            Err(_) => return timed_out(compaction),
        },
        None => sending.await,
    };
    let Ok(response) = sent else {
        warn!(
            template = route.template,
            "model proxy: upstream unreachable"
        );
        if deadline.is_some() {
            return error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "vibe_work_attachment_upstream_unreachable",
                "The attachment or compaction request could not reach Mistral. The turn was stopped; try again later.",
            );
        }
        return error(
            StatusCode::BAD_GATEWAY,
            "vibe_work_upstream_unreachable",
            "Vibe Work could not reach Mistral. Check the network or proxy settings.",
        );
    };
    relay_response(&shared, route, response, deadline, compaction).await
}

/// Relay a complete compaction answer or stream the ordinary endpoint's answer.
async fn relay_response(
    shared: &Shared,
    route: &RouteSpec,
    response: reqwest::Response,
    deadline: Option<tokio::time::Instant>,
    compaction: bool,
) -> Response {
    shared.stats.forwarded.fetch_add(1, Ordering::Relaxed);
    debug!(
        template = route.template,
        status = response.status().as_u16(),
        "model proxy: forwarded"
    );
    // Only the five rate-limit header names are read (never a body, never other headers).
    if let Some(limits) = RateLimits::from_headers(response.headers(), now_ms()) {
        (shared.config.on_rate_limits)(limits);
    }
    let upstream_status = response.status();
    if deadline.is_some()
        && (upstream_status == StatusCode::REQUEST_TIMEOUT
            || upstream_status == StatusCode::TOO_MANY_REQUESTS
            || upstream_status.is_server_error())
    {
        let request = if compaction {
            "compaction"
        } else {
            "attachment"
        };
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "vibe_work_model_upstream_refused",
            &format!(
                "Mistral returned HTTP {} for this {request} request. The turn was stopped; try again later.",
                upstream_status.as_u16()
            ),
        );
    }
    let mut out = Response::builder().status(response.status());
    if let Some(out_headers) = out.headers_mut() {
        for (name, value) in response.headers() {
            if !dropped(name, false) {
                out_headers.append(name.clone(), value.clone());
            }
        }
    }
    // A nonstreaming compaction can receive headers and then stall in the body. Return only a
    // complete success or an explicit failure, so Vibe can release its session lock.
    if let (true, Some(deadline)) = (compaction, deadline) {
        let Ok(Ok(bytes)) = tokio::time::timeout_at(deadline, response.bytes()).await else {
            return compaction_timeout();
        };
        return out
            .body(Body::from(bytes))
            .unwrap_or_else(|_| unreachable!());
    }
    if let Some(deadline) = deadline {
        let stream = response.bytes_stream();
        let bounded =
            futures_util::stream::unfold((stream, false), move |(mut stream, done)| async move {
                if done {
                    return None;
                }
                match tokio::time::timeout_at(deadline, stream.next()).await {
                    Ok(Some(Ok(bytes))) => Some((Ok(bytes), (stream, false))),
                    Ok(Some(Err(error))) => {
                        Some((Err(std::io::Error::other(error)), (stream, true)))
                    }
                    Ok(None) => None,
                    Err(_) => Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "attachment model response exceeded its deadline",
                        )),
                        (stream, true),
                    )),
                }
            });
        return out
            .body(Body::from_stream(bounded))
            .unwrap_or_else(|_| attachment_timeout());
    }
    // a successful connector catalog answer is read (bounded) to learn the connector aliases.
    if route.template == crate::connectors::BOOTSTRAP_TEMPLATE
        && response.status().is_success()
        && response
            .content_length()
            .is_none_or(|n| n <= crate::connectors::MAX_CATALOG as u64)
    {
        return relay_catalog(shared, out, response).await;
    }
    let stream = response.bytes_stream().map_err(std::io::Error::other);
    out.body(Body::from_stream(stream)).unwrap_or_else(|_| {
        error(
            StatusCode::BAD_GATEWAY,
            "vibe_work_upstream_unreachable",
            "Mistral's answer could not be passed on.",
        )
    })
}
