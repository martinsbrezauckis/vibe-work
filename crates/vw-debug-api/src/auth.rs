//! The request guard: Host, then Origin (and browser fetch metadata), then the bearer token.
//! Order from ShellX (`debug_api_auth.rs:323-345 @ e38c2b5c`): a browser or a
//! rebinding page gets 403 before the token is looked at, so 403 and 401 stay distinguishable.
//!
//! Depends on: axum (middleware), `reply` (error bodies), `server::ApiState`. Called by: the
//! router (a layer over every route; there are no exempt routes, `/health` included).
//! Invariants:
//! - Host: `127.0.0.1` or `localhost` (case-insensitive, one trailing dot allowed), with no port
//!   or exactly the bound port. IPv6 forms are refused: the server binds IPv4 loopback only. An
//!   absolute-form request target must pass the same rule.
//! - Origin: any `Origin` header must equal an allow-listed origin (empty list by default), so
//!   `Origin: null` and every web page are refused. A request with browser fetch metadata
//!   (`Sec-Fetch-Site` / `Sec-Fetch-Mode`) but no `Origin` is a browser no-cors request and is
//!   refused too.
//! - Token: exactly one `Authorization: Bearer <token>` header, compared in constant time. The
//!   token is never accepted from the query string (ShellX audit F-06).

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::reply::guard_error;
use crate::server::ApiState;

/// True when `value` (a `Host` header or URI authority) names our loopback listener.
#[must_use]
pub fn host_allowed(value: &str, port: u16) -> bool {
    let value = value.trim();
    if value.is_empty() || value.starts_with('[') {
        return false;
    }
    let (host, given_port) = match value.rsplit_once(':') {
        Some((host, p)) => (host, Some(p)),
        None => (value, None),
    };
    if given_port.is_some_and(|p| p != port.to_string()) {
        return false;
    }
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    host == "127.0.0.1" || host == "localhost"
}

/// True when every `Origin` header is allow-listed (or there is none) and a request without one
/// carries no browser fetch metadata.
#[must_use]
pub fn origin_allowed(headers: &HeaderMap, allowed: &[String]) -> bool {
    let mut origins = headers.get_all(header::ORIGIN).iter().peekable();
    if origins.peek().is_none() {
        return !headers.contains_key("sec-fetch-site") && !headers.contains_key("sec-fetch-mode");
    }
    origins.all(|origin| {
        origin
            .to_str()
            .is_ok_and(|o| allowed.iter().any(|a| a == o))
    })
}

/// Constant-time equality for equal-length inputs; the length itself is public (the token has a
/// fixed length), so the early length exit leaks nothing.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0_u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// True when the request carries exactly one `Authorization: Bearer <token>` with our token.
#[must_use]
pub fn token_present(headers: &HeaderMap, token: &str) -> bool {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        return false;
    };
    value
        .to_str()
        .ok()
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|given| constant_time_eq(given.as_bytes(), token.as_bytes()))
}

/// True when the request carries exactly one `Authorization: Bearer <token>` that matches the
/// store's current token.
pub(crate) fn token_matches(headers: &HeaderMap, store: &crate::token::TokenStore) -> bool {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        return false;
    };
    value
        .to_str()
        .ok()
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|given| store.matches(given.as_bytes()))
}

/// The middleware: refuses with 403 (host, origin) or 401 (token) before any route runs.
pub(crate) async fn guard(
    State(state): State<ApiState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let headers = request.headers();
    let host_ok = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| host_allowed(h, state.port))
        && request
            .uri()
            .authority()
            .is_none_or(|a| host_allowed(a.as_str(), state.port));
    if !host_ok {
        return guard_error(
            StatusCode::FORBIDDEN,
            "hostNotAllowed",
            "the Host must be this loopback listener",
        );
    }
    if !origin_allowed(headers, &state.allowed_origins) {
        return guard_error(
            StatusCode::FORBIDDEN,
            "originNotAllowed",
            "browser origins may not call the Debug API",
        );
    }
    if !token_matches(headers, &state.token) {
        let mut response = guard_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid bearer token (read it from the token file named in the descriptor)",
        );
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Bearer"),
        );
        return response;
    }
    next.run(request).await.into_response()
}
