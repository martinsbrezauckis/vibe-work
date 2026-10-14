//! HTTP answers: success bodies, the one error body shape and the status code of each error.
//!
//! Depends on: axum, vw-core-api (`CoreError`, `ErrorCode`). Called by: `routes`, `auth`, `sse`.
//! Invariants:
//! - Every error body is `{"error": {"code", "message",...}}` (ShellX shape);
//!   core errors keep their `method`, `summary` and `serverCode` fields.
//! - Status codes: guard refusals 403/401; gate refusals 403 (denied, token refused, declined),
//!   400 (invalid params or command), 428 (confirmation required), 409 (callback refused, no
//!   confirmation applicable), 404 (not loaded), 503/504/502 for server unavailable / timeout /
//!   server error, 500 internal.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::json;
use vw_core_api::{CoreError, ErrorCode};

/// A refusal by the request guard (no core involved).
pub(crate) fn guard_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"code": code, "message": message}})),
    )
        .into_response()
}

/// The status code for a core error.
#[must_use]
pub fn status_of(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::Denied | ErrorCode::ConfirmationRejected | ErrorCode::ConfirmationDeclined => {
            StatusCode::FORBIDDEN
        }
        ErrorCode::InvalidParams | ErrorCode::InvalidCommand => StatusCode::BAD_REQUEST,
        // `confirmDeletes`: name the files in `deletePaths` and send again.
        ErrorCode::ConfirmationRequired | ErrorCode::ConfirmDeletes => {
            StatusCode::PRECONDITION_REQUIRED
        }
        // `keyService`: a key/vault operation was refused (locked, wrong passphrase, two vaults,
        // invalid key,...); the reason word is in `serverCode`.
        ErrorCode::ConfirmationNotApplicable
        | ErrorCode::KeyService
        | ErrorCode::VibeInstall
        | ErrorCode::CallbackRefused
        | ErrorCode::SessionStuck
        | ErrorCode::Cancelled
        | ErrorCode::Stale => StatusCode::CONFLICT,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        // `accessModeUnverified` is retryable: the next command for the session opens and restores
        // it again.
        ErrorCode::ServerUnavailable | ErrorCode::AccessModeUnverified => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
        ErrorCode::ServerError => StatusCode::BAD_GATEWAY,
        ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// A core error as an HTTP answer.
pub(crate) struct ApiError(pub(crate) CoreError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (status_of(self.0.code), Json(json!({"error": self.0}))).into_response()
    }
}

impl From<CoreError> for ApiError {
    fn from(error: CoreError) -> Self {
        Self(error)
    }
}

/// A success body.
pub(crate) fn ok<T: Serialize>(body: &T) -> Response {
    match serde_json::to_value(body) {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(_) => guard_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "the answer could not be encoded",
        ),
    }
}
