//! The one choke point of the secret rule: no Debug API answer carries a stored secret value.
//!
//! the Debug API controls the whole app as the user, but "you can't get secrets from the
//! store as values". Every route is open, so instead of trusting each one, every answer is searched for every
//! value in `vw_secrets` (the Mistral key, vault values, connector credentials, the Debug API and proxy tokens)
//! before it leaves, and each occurrence is replaced by `[secret withheld: <kind>]`: JSON and text bodies here
//! ([`guard_secrets`], a layer around the whole router, `ui/*` replies from the webview included), SSE events in
//! `sse.rs` ([`scrub_event`]).
//!
//! Depends on: axum, vw-secrets, tracing. Called by: `server` (the layer), `sse` (each event).
//! Invariants:
//! - A hit keeps the answer (a session whose agent printed the proxy token stays readable) with every occurrence
//!   replaced, adds the header `x-vw-secret-withheld: <kinds>` and logs the route and the kinds, never the value.
//!   The marker has no quotes or backslashes, so a JSON body stays valid JSON.
//! - Bodies are buffered up to [`MAX_CHECKED_BYTES`]; a larger body is refused (`500 secretWithheld`, fail
//!   closed). `Content-Length` is recomputed by the server from the new body.
//! - Not searched: `image/*` bodies (window and preview screenshots: pixels, not text; the key screens never
//!   draw a key) and the SSE stream as a whole (its events are checked one by one).

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tracing::warn;
use vw_secrets::SecretKind;

/// Largest body that is searched; a larger one is refused (no Debug API answer comes close).
pub const MAX_CHECKED_BYTES: usize = 256 * 1024 * 1024;
/// The header that says an answer was scrubbed (kinds, comma separated; never values).
pub const WITHHELD_HEADER: &str = "x-vw-secret-withheld";

fn kind_list(kinds: &[SecretKind]) -> String {
    kinds
        .iter()
        .map(|k| k.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// The layer: runs the route, then scrubs its body (module docs).
pub(crate) async fn guard_secrets(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let response = next.run(request).await;
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.starts_with("text/event-stream") || content_type.starts_with("image/") {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_CHECKED_BYTES).await else {
        warn!(route = %path, "Debug API answer refused: too large to check for secret values");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"error": {
                "code": "secretWithheld",
                "message": "the answer was too large to check for stored secret values",
            }})),
        )
            .into_response();
    };
    let Some((scrubbed, kinds)) = vw_secrets::scrub(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let list = kind_list(&kinds);
    warn!(route = %path, kinds = %list, "Debug API answer scrubbed: it held a stored secret value");
    parts.headers.remove(header::CONTENT_LENGTH);
    if let Ok(value) = HeaderValue::from_str(&list) {
        parts.headers.insert(WITHHELD_HEADER, value);
    }
    Response::from_parts(parts, Body::from(scrubbed))
}

/// One SSE event's JSON with every stored secret value replaced (module docs); unchanged when none is found.
pub(crate) fn scrub_event(json: String) -> String {
    match vw_secrets::scrub(json.as_bytes()) {
        None => json,
        Some((bytes, kinds)) => {
            warn!(kinds = %kind_list(&kinds), "Debug API event scrubbed: it held a stored secret value");
            String::from_utf8(bytes).unwrap_or_else(|_| {
                json!({"withheld": "the event held a stored secret value"}).to_string()
            })
        }
    }
}
