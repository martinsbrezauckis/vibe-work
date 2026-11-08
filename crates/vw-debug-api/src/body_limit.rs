//! Structured HTTP body-limit errors for every Debug API route, including encoded file uploads.
//! Depends on axum, config and reply; server installs this inside authentication and secret filtering.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::FILES_BODY_LIMIT;
use crate::reply::guard_error;
use crate::server::ApiState;

pub(crate) async fn structured(
    State(state): State<ApiState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let limit = if request.uri().path() == "/ui/files" {
        FILES_BODY_LIMIT
    } else {
        state.body_limit
    };
    let response = next.run(request).await;
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        guard_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payloadTooLarge",
            &format!(
                "request body exceeds the {limit}-byte limit (including base64 and JSON); send a smaller request"
            ),
        )
    } else {
        response
    }
}
