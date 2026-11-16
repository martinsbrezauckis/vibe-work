//! The MCP forwarder: custom MCP servers that need an API key. Vibe's config names the server at
//! `http://127.0.0.1:<proxy port>/vw-mcp/<binding id>` with `api_key_env = "VW_MCP_TOKEN"`, so Vibe sends the
//! proxy token (never a key); this module checks it, asks the core's [`McpResolver`] for the binding (the upstream
//! address the user confirmed, the header, the key from the vault, the user's grant), and forwards the request with
//! the key added.
//!
//! Request order (each step refuses before anything leaves the machine):
//! 1. the path is exactly `/vw-mcp/<id>` (`id`: 1 to 64 of `a-z 0-9 -`), no query → else 404 / 400;
//! 2. `Authorization: Bearer <token>` is the current proxy token → else 401 (`vibe_work_proxy_token`);
//! 3. the resolver answers the binding (it may first wait while the person answers the core's first-use key dialog):
//!    unknown → 404, no grant → 403 (`vibe_work_mcp_not_allowed`), vault locked
//!    or secret gone → 503 at once (Vibe gives a server 10 s to start, `startup_timeout_sec`), no resolver → 503;
//! 4. the request goes to the binding's upstream (never an address from the request) with hop-by-hop headers,
//!    `Host`, `Authorization`, `Cookie`, `X-Api-Key` and the binding's header removed, the key header added
//!    (marked sensitive), the body streamed;
//! 5. a 3xx answer is refused with 502 (`vibe_work_mcp_redirect`) instead of being passed back for the client to
//!    follow, so a request carrying the key only ever goes to the address the user confirmed; a compressed answer
//!    (any `Content-Encoding` but `identity`; the request goes without `Accept-Encoding`) is refused with 502
//!    (`vibe_work_mcp_encoded`), because it cannot be checked for the key; an answer with a header name or value
//!    that holds the inserted key in any form the scrub knows (`vw_secrets::holds`) is refused with 502
//!    (`vibe_work_mcp_header_key`), because a client can copy a header into what the agent reads (an unexpected
//!    `Content-Type` becomes a tool error). When the request was one JSON-RPC `tools/call` (its body read whole, up
//!    to `CALL_PEEK` bytes, before forwarding), each of these three refusals is instead that call's answer: a tool
//!    result with `isError: true` whose text is the refusal's sentence, because Vibe's MCP client gives the agent
//!    such a result's text but turns any HTTP error into a fixed sentence. Anything else is streamed back (status, headers minus hop-by-hop and
//!    `Content-Length`, body) through `vw_secrets::StreamScrub`: the key this forwarder inserted (at any length, the
//!    registry's minimum aside) and every other registered stored value are withheld, also
//!    when the server splits one across two parts of its answer. The person allowed the key to go to this server,
//!    not to the agent that reads the server's answers.
//!
//! The client is the proxy's own (`server.rs`: `vw_mistral::proxy_client()`, built with
//! `reqwest::redirect::Policy::none()` in `vw-mistral/src/client.rs`): a following client would never show the 3xx
//! to step 5 and would carry the key header to the redirect target. A `Location` header is never passed back on any
//! answer (MCP answers do not use it), so nothing can tell the client to repeat the request elsewhere.
//!
//! Depends on: axum, reqwest (the proxy's no-redirect client), futures-util, zeroize, vw-secrets (the answer scrub).
//! Called by: `server`.
//! Invariants: never logs a header value, the key, the token or the upstream address (binding id and status only);
//! the key exists only in the resolver's answer, the outgoing header and memory that is zeroized on drop.

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use serde_json::json;
use tracing::{debug, warn};
use zeroize::Zeroizing;

/// Path prefix of forwarded MCP servers.
pub const MCP_PREFIX: &str = "/vw-mcp/";

/// Shared MCP requests cannot prove which chat is using the Debug bearer.
#[must_use]
pub fn debug_token_chat_required() -> String {
    format!(
        "Not run: shared MCP connections have no chat mode. Use the Debug API token through secret_run in a {} chat.",
        vw_vocabulary::label("accessModes", "auto-approve"),
    )
}

/// Longest binding id.
const MAX_ID: usize = 64;

/// Where one binding goes and with what (the resolver's answer). `Debug` shows neither address nor key.
pub struct McpTarget {
    /// The upstream server address (`https://...`, or `http://` to this computer), as the user confirmed it.
    pub upstream: String,
    /// The header that carries the key (`authorization` or a custom name).
    pub header: String,
    /// The header's whole value (`Bearer <key>` or the key as is).
    pub value: Zeroizing<String>,
}

impl std::fmt::Debug for McpTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("McpTarget(<redacted>)")
    }
}

/// Why a binding cannot be used now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpRefusal {
    /// No such binding.
    Unknown,
    /// The user has not allowed the key for this server (or revoked it).
    NotAllowed,
    /// A policy record is missing, corrupt or undecodable.
    PermissionsUnreadable,
    /// The vault is locked.
    Locked,
    /// The user paused agent access while the model key proxy remains available.
    AgentAccessLocked,
    /// The secret is gone or unreadable.
    SecretMissing,
    /// The Debug bearer needs an identified Full access chat, which a shared connection cannot supply.
    DebugTokenRequiresChat,
}

/// What [`McpResolver::resolve`] returns.
pub type McpFuture<'a> = BoxFuture<'a, Result<McpTarget, McpRefusal>>;

/// The core's side: answers a binding id with its target, or why not. Called once per forwarded request.
pub trait McpResolver: Send + Sync {
    /// The target of binding `id`.
    fn resolve<'a>(&'a self, id: &'a str) -> McpFuture<'a>;
}

/// The slot the core fills after it starts (`None`: every MCP request answers 503).
pub type McpResolverSlot = Arc<RwLock<Option<Arc<dyn McpResolver>>>>;

fn error(status: StatusCode, kind: &str, message: &str) -> Response {
    let body = serde_json::json!({"error": {"type": kind, "message": message}, "object": "error"});
    Response::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// The binding id of `path` when it is exactly `/vw-mcp/<id>`.
fn binding_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix(MCP_PREFIX)?;
    (!id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    .then_some(id)
}

/// Headers never copied in either direction (hop-by-hop); toward the upstream also the ones that could carry Vibe's
/// token or the user's cookies, the binding's own key header and `Accept-Encoding` (the answer must be checkable);
/// toward the client also `Location` and `Content-Length` (the scrubbed body can differ in length; module docs).
fn dropped(name: &HeaderName, request: bool, key_header: &str) -> bool {
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
    ) || (!request && matches!(name.as_str(), "location" | "content-length"))
        || (request
            && (matches!(
                name.as_str(),
                "host" | "authorization" | "cookie" | "x-api-key" | "accept-encoding"
            ) || name.as_str().eq_ignore_ascii_case(key_header)))
}

fn refusal_response(refusal: McpRefusal) -> Response {
    match refusal {
        McpRefusal::Unknown => error(
            StatusCode::NOT_FOUND,
            "vibe_work_mcp_unknown",
            "Vibe Work has no MCP server with a key at this address. Add it again in Vibe Work's Tools page.",
        ),
        McpRefusal::NotAllowed => error(
            StatusCode::FORBIDDEN,
            "vibe_work_mcp_not_allowed",
            "This MCP server's key is not allowed now. In Vibe Work, open Tools and choose Allow key.",
        ),
        McpRefusal::PermissionsUnreadable => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "vibe_work_mcp_permissions_unreadable",
            "The secret's permissions could not be read; forwarding was refused. Ask the user to check the vault in Vibe Work.",
        ),
        McpRefusal::Locked => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "vibe_work_key_unavailable",
            vw_vocabulary::texts::AGENT_VAULT_LOCKED,
        ),
        McpRefusal::AgentAccessLocked => error(
            StatusCode::FORBIDDEN,
            "vibe_work_agent_vault_locked",
            vw_vocabulary::text("agentPausedRefusal").unwrap_or("agentPausedRefusal"),
        ),
        McpRefusal::SecretMissing => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "vibe_work_mcp_secret_missing",
            "The secret this MCP server uses is no longer in Vibe Work's vault. Add the server again.",
        ),
        McpRefusal::DebugTokenRequiresChat => error(
            StatusCode::FORBIDDEN,
            "vibe_work_debug_token_requires_chat",
            &debug_token_chat_required(),
        ),
    }
}

/// True when `headers` carry `token` as `Authorization: Bearer <token>` and it is the current one.
fn token_ok(headers: &HeaderMap, accepts: &(dyn Fn(&str) -> bool + Sync)) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| accepts(t.trim()))
}

/// Handles one request on [`MCP_PREFIX`] (module docs for the order).
pub(crate) async fn handle(
    client: &reqwest::Client,
    resolver: &McpResolverSlot,
    accepts: &(dyn Fn(&str) -> bool + Sync),
    parts: Parts,
    body: Body,
) -> Response {
    let Some(id) = binding_id(parts.uri.path()) else {
        return error(
            StatusCode::NOT_FOUND,
            "vibe_work_mcp_unknown",
            "Vibe Work's MCP forwarder has no such address.",
        );
    };
    if parts.uri.query().is_some() {
        return error(
            StatusCode::BAD_REQUEST,
            "vibe_work_mcp_query",
            "Vibe Work's MCP forwarder takes no query.",
        );
    }
    if !token_ok(&parts.headers, accepts) {
        warn!("mcp forwarder: refused a request without the current token");
        return error(
            StatusCode::UNAUTHORIZED,
            "vibe_work_proxy_token",
            "Vibe Work's MCP forwarder refused this request: it does not carry the current proxy token.",
        );
    }
    let current = resolver
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Some(current) = current else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "vibe_work_mcp_not_ready",
            "Vibe Work is still starting. Refresh the server in a moment.",
        );
    };
    let short: String = id.chars().take(24).collect();
    let target = match current.resolve(id).await {
        Ok(target) => target,
        Err(refusal) => {
            debug!(binding = %short, ?refusal, "mcp forwarder: refused");
            return refusal_response(refusal);
        }
    };
    forward(client, &short, &target, parts, body).await
}

async fn forward(
    client: &reqwest::Client,
    short: &str,
    target: &McpTarget,
    parts: Parts,
    body: Body,
) -> Response {
    let (Ok(name), Ok(mut value)) = (
        HeaderName::from_bytes(target.header.as_bytes()),
        HeaderValue::from_str(&target.value),
    ) else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "vibe_work_mcp_secret_missing",
            "The stored key contains characters a header cannot carry. Ask the user to replace the secret in Vibe Work.",
        );
    };
    value.set_sensitive(true);
    // Registered before anything is sent (the Debug API's answers withhold it too); this answer's own scrub withholds
    // it at any length (module docs, step 5).
    let key = target
        .value
        .strip_prefix("Bearer ")
        .unwrap_or(&target.value);
    vw_secrets::register(vw_secrets::SecretKind::VaultValue, key);
    let mut headers = reqwest::header::HeaderMap::new();
    for (header, v) in &parts.headers {
        if !dropped(header, true, &target.header) {
            headers.append(header.clone(), v.clone());
        }
    }
    headers.insert(name, value);
    let (call, body) = peek_call(body).await;
    let sent = client
        .request(parts.method.clone(), target.upstream.as_str())
        .headers(headers)
        .body(body)
        .send()
        .await;
    let Ok(response) = sent else {
        warn!(binding = %short, "mcp forwarder: upstream unreachable");
        return error(
            StatusCode::BAD_GATEWAY,
            "vibe_work_upstream_unreachable",
            "Vibe Work could not reach this MCP server.",
        );
    };
    let status = response.status();
    debug!(binding = %short, status = status.as_u16(), "mcp forwarder: forwarded");
    if status.is_redirection() {
        warn!(binding = %short, status = status.as_u16(), "mcp forwarder: refused a redirect");
        return refused_answer(
            call.as_ref(),
            "vibe_work_mcp_redirect",
            "This MCP server answered with a redirect. Vibe Work sends your key only to the address you confirmed: add the server with its final address.",
        );
    }
    let encoded = response
        .headers()
        .get_all(axum::http::header::CONTENT_ENCODING)
        .iter()
        .any(|v| !v.as_bytes().eq_ignore_ascii_case(b"identity"));
    if encoded {
        warn!(binding = %short, "mcp forwarder: refused a compressed answer");
        return refused_answer(
            call.as_ref(),
            "vibe_work_mcp_encoded",
            "This MCP server sent a compressed answer, which Vibe Work cannot check for your key. Turn compression off for this server.",
        );
    }
    // Header names arrive lower-cased: they are searched for the key's forms and the lower-cased key's (its hex
    // forms differ). A lower-cased base64 form cannot be told from other text (`vw-secrets`: transformed values).
    let lower = Zeroizing::new(key.to_ascii_lowercase());
    if response.headers().iter().any(|(name, v)| {
        let name = name.as_str().as_bytes();
        vw_secrets::holds(name, key)
            || vw_secrets::holds(name, &lower)
            || vw_secrets::holds(v.as_bytes(), key)
    }) {
        warn!(binding = %short, "mcp forwarder: refused an answer header holding the key");
        return refused_answer(
            call.as_ref(),
            "vibe_work_mcp_header_key",
            "This MCP server sent your key back in a header of its answer, so Vibe Work did not pass the answer on.",
        );
    }
    let mut out = Response::builder().status(status);
    if let Some(out_headers) = out.headers_mut() {
        for (header, v) in response.headers() {
            if !dropped(header, false, "") {
                out_headers.append(header.clone(), v.clone());
            }
        }
    }
    out.body(Body::from_stream(scrubbed(response, key)))
        .unwrap_or_else(|_| {
            error(
                StatusCode::BAD_GATEWAY,
                "vibe_work_upstream_unreachable",
                "The MCP server's answer could not be passed on.",
            )
        })
}

/// The longest request body read before forwarding to learn whether it is one tool call. MCP requests are small;
/// a longer body is forwarded as it streams, and its refusals stay HTTP errors.
const CALL_PEEK: usize = 1 << 20;

/// The JSON-RPC id of `body` when it is one `tools/call` request.
fn tool_call_id(body: &[u8]) -> Option<serde_json::Value> {
    let request: serde_json::Value = serde_json::from_slice(body).ok()?;
    let id = request.get("id")?;
    (request.get("method")?.as_str()? == "tools/call" && (id.is_number() || id.is_string()))
        .then(|| id.clone())
}

/// Reads up to [`CALL_PEEK`] bytes of the request body: the tool call's id when the whole body was read, and the
/// body to forward, unchanged (the parts read, then the rest as it streams).
async fn peek_call(body: Body) -> (Option<serde_json::Value>, reqwest::Body) {
    let mut rest = body.into_data_stream();
    let mut read = Vec::new();
    let mut head = Vec::new();
    let mut whole = true;
    while let Some(part) = rest.next().await {
        let fits = match &part {
            Ok(bytes) => {
                head.extend_from_slice(bytes);
                head.len() <= CALL_PEEK
            }
            Err(_) => false,
        };
        read.push(part);
        if !fits {
            whole = false;
            break;
        }
    }
    let call = if whole { tool_call_id(&head) } else { None };
    let body = futures_util::stream::iter(read).chain(rest);
    (call, reqwest::Body::wrap_stream(body))
}

/// A refusal of the server's answer (module docs, step 5): for a tool call, that call's tool result with `isError`
/// and the refusal's sentence (the kind in `_meta`); otherwise the 502 error. Neither holds anything from the answer.
fn refused_answer(call: Option<&serde_json::Value>, kind: &str, message: &str) -> Response {
    let Some(id) = call else {
        return error(StatusCode::BAD_GATEWAY, kind, message);
    };
    let body = json!({"jsonrpc": "2.0", "id": id, "result": {
        "content": [{"type": "text", "text": message}],
        "isError": true,
        "_meta": {"vibe-work/refusal": kind},
    }});
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|_| error(StatusCode::BAD_GATEWAY, kind, message))
}

/// The upstream body with every registered stored value and the inserted `key` (at any length) withheld, a value
/// split across two parts included (`vw_secrets::StreamScrub`); the kept tail is passed on when the body ends.
fn scrubbed(
    response: reqwest::Response,
    key: &str,
) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send + use<> {
    let state = (
        response.bytes_stream(),
        Some(vw_secrets::StreamScrub::with_value(
            vw_secrets::SecretKind::VaultValue,
            key,
        )),
    );
    futures_util::stream::unfold(state, |(mut upstream, scrub)| async move {
        let mut scrub = scrub?;
        match upstream.next().await {
            Some(Ok(part)) => {
                let passed = scrub.push(&part);
                Some((Ok(passed.into()), (upstream, Some(scrub))))
            }
            Some(Err(problem)) => Some((Err(std::io::Error::other(problem)), (upstream, None))),
            None => Some((Ok(scrub.finish().into()), (upstream, None))),
        }
    })
}
