//! Connector request and answer bodies at the proxy: the gateway `POST` and the credential write are read and judged
//! before the key is added (`connectors.rs`, `credentials.rs`); a catalog answer teaches the aliases and the connectable
//! connectors and gains missing tool descriptions on its way back. Every other body streams through unread.
//!
//! Depends on: axum, reqwest, `connectors`, `credentials`, `server::error`. Called by: `server`.

use axum::body::Body;
use axum::http::StatusCode;
use axum::http::header::{CONTENT_LENGTH, HeaderValue};
use axum::response::{IntoResponse, Response};

use crate::connectors::Aliases;
use crate::credentials::Connectable;
use crate::routes::RouteSpec;
use crate::server::error;

/// The body to send upstream: streamed unread for every route but a `POST` to the connector gateway and the credential
/// write, which are read and judged first (`connectors.rs`, `credentials.rs`). `Err` = the answer to give instead.
pub(crate) async fn request_body(
    aliases: &Aliases,
    connectable: &Connectable,
    route: &RouteSpec,
    parts: &axum::http::request::Parts,
    body: Body,
) -> Result<reqwest::Body, Box<Response>> {
    if route.template == crate::credentials::CREDENTIALS_TEMPLATE {
        let read = axum::body::to_bytes(body, crate::credentials::MAX_CREDENTIALS_BODY).await;
        return match read {
            // `/v1/connectors/<id>/user/credentials`: the id is the fourth segment (the allowlist checked its shape).
            Ok(bytes)
                if crate::credentials::no_secret_connection(&bytes)
                    && connectable.take(parts.uri.path().split('/').nth(3).unwrap_or("")) =>
            {
                Ok(bytes.into())
            }
            _ => Err(Box::new(error(
                StatusCode::BAD_REQUEST,
                "vibe_work_connector_request",
                crate::credentials::REFUSED,
            ))),
        };
    }
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
    match crate::connectors::judge(&bytes, id, aliases) {
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
pub(crate) async fn relay_catalog(
    aliases: &Aliases,
    connectable: &Connectable,
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
    aliases.learn(&body);
    connectable.learn(&body);
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
