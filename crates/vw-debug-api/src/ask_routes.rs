//! Permission routes through the core's shared service. Depends on guarded `ApiState`; server merges this router.
//! Vault permissions always remain native and person-only.
//! `GET /asks` also lists the session grants Vibe Work keeps for Vibe's approvals ("Allow for this session");
//! `POST /asks/grants/{id}/remove` removes one, like the window's Remove (the Debug API acts as the user).
use crate::{
    reply::{ApiError, ok},
    routes::decode,
    server::ApiState,
};
use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
};

pub(crate) fn router() -> Router<ApiState> {
    Router::new()
        .route("/asks", get(list))
        .route("/asks/{id}", post(answer))
        .route("/asks/grants/{id}/remove", post(remove_grant))
}
async fn list(State(state): State<ApiState>) -> Response {
    ok(
        &serde_json::json!({"open":state.core.asks(), "records":state.core.ask_records(),
        "grants":state.core.ask_session_grants()}),
    )
}
async fn remove_grant(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    let request = vw_core_api::Command::AskGrants {
        request: vw_core_api::AskGrantsRequest::Remove { id },
    };
    match state
        .core
        .execute(vw_core_api::Origin::DebugApi, request)
        .await
    {
        Ok(result) => ok(&result),
        Err(e) => ApiError(e).into_response(),
    }
}
async fn answer(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let answer: vw_ask::Answer = match decode(&headers, &body) {
        Ok(a) => a,
        Err(response) => return *response,
    };
    match state.core.answer_ask(&id, answer).await {
        Ok(()) => ok(&serde_json::json!({"ok":true})),
        Err(e) => ApiError(e).into_response(),
    }
}
