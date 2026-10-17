//! Read-only UI command trace adapter. Depends on the guarded UI registry; called by the Debug API router.
//! Only the window holds the bounded metadata buffer. Reading it never executes a core command.

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use vw_core_api::{CoreError, ErrorCode};

use crate::reply::{ApiError, ok};
use crate::server::ApiState;

#[derive(Default, Deserialize)]
pub(crate) struct TraceQuery {
    #[serde(default)]
    since: u64,
}

pub(crate) async fn trace(
    State(state): State<ApiState>,
    query: Result<Query<TraceQuery>, QueryRejection>,
) -> Response {
    let query = match query {
        Ok(Query(query)) if query.since <= 9_007_199_254_740_991 => query,
        _ => {
            return ApiError(CoreError::new(
                ErrorCode::InvalidParams,
                "since must be a non-negative safe integer",
            ))
            .into_response();
        }
    };
    let Some(handler) = state.ui_routes.get("trace") else {
        return ApiError(CoreError::new(
            ErrorCode::NotFound,
            "the UI trace is unavailable",
        ))
        .into_response();
    };
    match handler(json!({"since": query.since})).await {
        Ok(value) => ok(&value),
        Err(error) => ApiError(error).into_response(),
    }
}
