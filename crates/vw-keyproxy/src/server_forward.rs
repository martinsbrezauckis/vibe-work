//! Per-request model proxy admission, key wait, forwarding and bounded response streaming.
//! Uses the server's shared state and existing route/body policies; called by its Axum fallback.
//! Admission order, deadlines, response text and credential handling are unchanged.
use super::Shared;
use super::resume::admit as resume_admission;
use crate::compaction::{MAX_CHAT_BODY, prepare};
use crate::key::{KeyNeed, KeyState};
use crate::limits::{RateLimits, now_ms};
use crate::mcp::MCP_PREFIX;
use crate::routes::{RouteSpec, Upstream, match_route};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{ACCEPT_ENCODING, AUTHORIZATION, CONTENT_LENGTH, HeaderName, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{StreamExt, TryStreamExt};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{debug, warn};

pub(crate) fn error(status: StatusCode, kind: &str, message: &str) -> Response {
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

fn stopping(shared: &Shared) -> Refusal {
    shared.stats.stopping.fetch_add(1, Ordering::Relaxed);
    refuse(
        StatusCode::UNPROCESSABLE_ENTITY,
        "vibe_work_proxy_stopping",
        "Vibe Work is closing. This model request was stopped; try again after restart.",
    )
}

/// Step 3: the key as a sensitive `Authorization` value, waiting for it when needed.
///
/// `wait` false (startup discovery): one snapshot of the key decides, never a wait and never the owner's
/// hook. Vibe fetches the catalog when a chat starts and gives it 30 s; held for a key, it outlasted the chat start (a
/// new chat with the vault locked or no key timed out and stuck). The 503 instead makes Vibe start the chat without
/// connectors, and it caches only a successful catalog, so the next chat after an unlock loads them. One snapshot, not
/// a check before a wait: a vault that locks between the two can no longer send the catalog into the wait.
async fn authorization(shared: &Shared, wait: bool) -> Result<HeaderValue, Refusal> {
    let mut stopping_rx = shared.stopping.clone();
    if *stopping_rx.borrow() {
        return Err(stopping(shared));
    }
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
        tokio::select! {
            biased;
            _ = stopping_rx.wait_for(|stopping| *stopping) => return Err(stopping(shared)),
            result = shared.key.wait_ready(shared.config.key_wait, move |need| {
                stats.waited.fetch_add(1, Ordering::Relaxed);
                hook(need);
            }) => result.map_err(unavailable)?,
        }
    } else {
        match shared.key.current() {
            KeyState::Ready(key) => key,
            KeyState::Missing(need) => return Err(unavailable(need)),
        }
    };
    if *stopping_rx.borrow() {
        return Err(stopping(shared));
    }
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

struct PreparedBody {
    identity: Option<(Option<String>, Option<String>)>,
    body: reqwest::Body,
    deadline: Option<tokio::time::Instant>,
    compaction: bool,
    /// A side chat request (`ChatShape::side`): its answer's rate limits are not offered.
    side: bool,
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
        } else if shape.new_attachment {
            Some(arrived + shared.config.image_deadline)
        } else {
            None
        };
        let rewritten_len = rewritten.as_ref().map(Vec::len);
        let body = rewritten.map_or_else(|| reqwest::Body::from(bytes), reqwest::Body::from);
        Ok(PreparedBody {
            identity: Some((shape.session, shape.input)),
            body,
            deadline,
            compaction: shape.compaction,
            side: shape.side,
            rewritten_len,
        })
    } else {
        let body = crate::connector_bodies::request_body(
            &shared.aliases,
            &shared.connectable,
            route,
            parts,
            body,
        )
        .await?;
        Ok(PreparedBody {
            identity: None,
            body,
            deadline: None,
            compaction: false,
            side: false,
            rewritten_len: None,
        })
    }
}

/// The upstream base URL of `route`.
fn upstream_base<'a>(shared: &'a Shared, route: &RouteSpec) -> &'a str {
    match route.upstream {
        Upstream::Api => &shared.config.upstreams.api,
        Upstream::Console => &shared.config.upstreams.console,
        Upstream::Chat => &shared.config.upstreams.chat,
    }
}

/// Discovery falls back to local config without a key; an unlock wait only strands startup and cleanup.
fn startup_discovery(route: &RouteSpec) -> bool {
    matches!(
        route.template,
        crate::connectors::BOOTSTRAP_TEMPLATE
            | "/v1/users/me"
            | "/v1/billing/subscription"
            | "/v1/billing/budget"
            | "/console/api/vibe/whoami"
            | "/chat/api/v1/code/managed-config"
    )
}

pub(super) async fn handle(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    let arrived = tokio::time::Instant::now();
    let (parts, body) = request.into_parts();
    // MCP servers use their own route and credentials.
    if parts.uri.path().starts_with(MCP_PREFIX) {
        let tokens = Arc::clone(&shared.tokens);
        let accepts = move |t: &str| tokens.accepts(t);
        let (client, mcp) = (shared.client.get(), &shared.config.mcp);
        return crate::mcp::handle(&client, mcp, &accepts, parts, body).await;
    }
    let route = match admit(&shared, &parts) {
        Ok(route) => route,
        Err(r) => return error(r.status, r.kind, r.message),
    };
    let wait = !startup_discovery(route);
    // connector calls are checked before the key is used.
    let PreparedBody {
        identity,
        body,
        deadline,
        compaction,
        side,
        rewritten_len,
    } = match prepared_body(&shared, route, &parts, body, arrived).await {
        Ok(prepared) => prepared,
        Err(answer) => return *answer,
    };
    if let Err(answer) = resume_admission(&shared, identity.as_ref()).await {
        return *answer;
    }
    let auth = match bounded_authorization(&shared, wait, deadline, compaction).await {
        Ok(auth) => auth,
        Err(answer) => return *answer,
    };
    // Recheck after key waits: a barrier or token rotation can happen while the vault is locked.
    if let Err(r) = admit(&shared, &parts) {
        return error(r.status, r.kind, r.message);
    }
    if let Err(answer) = resume_admission(&shared, identity.as_ref()).await {
        return *answer;
    }
    let base = upstream_base(&shared, route);
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
        .get()
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
    relay_response(&shared, route, response, deadline, compaction, side).await
}

async fn bounded_authorization(
    shared: &Shared,
    wait: bool,
    deadline: Option<tokio::time::Instant>,
    compaction: bool,
) -> Result<HeaderValue, Box<Response>> {
    match deadline {
        Some(deadline) => {
            match tokio::time::timeout_at(deadline, authorization(shared, wait)).await {
                Ok(Ok(auth)) => Ok(auth),
                Ok(Err(r)) => Err(Box::new(error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    r.kind,
                    r.message,
                ))),
                Err(_) => Err(Box::new(timed_out(compaction))),
            }
        }
        None => authorization(shared, wait)
            .await
            .map_err(|r| Box::new(error(r.status, r.kind, r.message))),
    }
}

/// Relay a complete compaction answer or stream the ordinary endpoint's answer.
async fn relay_response(
    shared: &Shared,
    route: &RouteSpec,
    response: reqwest::Response,
    deadline: Option<tokio::time::Instant>,
    compaction: bool,
    side: bool,
) -> Response {
    shared.stats.forwarded.fetch_add(1, Ordering::Relaxed);
    debug!(
        template = route.template,
        status = response.status().as_u16(),
        "model proxy: forwarded"
    );
    // Only the five rate-limit header names are read (never a body, never other headers). Mistral's limits are per
    // model: a side request's answer (title, classifier, summary) is another model's and keeps the chat's readout.
    if !side && let Some(limits) = RateLimits::from_headers(response.headers(), now_ms()) {
        shared.limits.offer(limits);
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
    if deadline.is_some() {
        let idle_limit = shared.config.image_deadline;
        let stream = response.bytes_stream();
        let bounded =
            futures_util::stream::unfold((stream, false), move |(mut stream, done)| async move {
                if done {
                    return None;
                }
                match tokio::time::timeout(idle_limit, stream.next()).await {
                    Ok(Some(Ok(bytes))) => Some((Ok(bytes), (stream, false))),
                    Ok(Some(Err(error))) => {
                        Some((Err(std::io::Error::other(error)), (stream, true)))
                    }
                    Ok(None) => None,
                    Err(_) => Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "attachment model response stopped producing bytes",
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
        return crate::connector_bodies::relay_catalog(
            &shared.aliases,
            &shared.connectable,
            out,
            response,
        )
        .await;
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
