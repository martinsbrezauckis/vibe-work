//! Routes v1. Every handler is a thin adapter over `Core::execute` (or a read of core state), so
//! the Debug API can do exactly what the UI can, through the same gate, never around it.
//!
//! Depends on: axum, vw-core-api, `reply`, `sse`, `registry`, `server::ApiState`. Called by:
//! `server` (builds the router).
//! Routes: `GET /health`, `GET /sessions`, `GET /sessions/{id}`, `GET /sessions/{id}/anomalies`,
//! `GET /sessions/{id}/processes` (read only,: listeners, owners, leases),
//! `GET /sessions/{id}/changes` (the Changes tab's view), `GET /events` (SSE), `POST /commands`,
//! `POST /callbacks/{id}`, `GET /server`, `GET /ui`, `POST /ui/{*name}`, plus the terminal routes merged
//! from `terminal_routes`. Anything else is a JSON 404.
//! Invariants:
//! - Request bodies are decoded by us with value-free error texts (`invalid_command`) and must be
//!   `application/json`, so a form post cannot reach a command even if it got past the guard.
//! - No route returns a secret: no handler reads the API key or the bearer token (the token is
//!   only compared in `auth`), and stderr lines arrive with the key already replaced.

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, CoreError, ErrorCode, Origin, invalid_command};

use crate::reply::{ApiError, guard_error, ok};
use crate::server::ApiState;
use crate::sse;

/// The v1 router, before the guard layer is added.
pub(crate) fn router() -> Router<ApiState> {
    let routes = Router::new();
    // Test builds only: the in-page channel for the end-to-end runs without a WebDriver. Merged here,
    // inside the guard and the secret guard the server adds around this router.
    #[cfg(feature = "e2e-script")]
    let routes = routes.merge(crate::e2e_script::router());
    // Private test builds only: the vault test switch, inside the same guards.
    #[cfg(feature = "vault-e2e")]
    let routes = routes.merge(crate::vault_e2e::router());
    routes
        .route("/health", get(health))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{id}", get(read_session))
        .route("/sessions/{id}/anomalies", get(read_anomalies))
        .route("/sessions/{id}/processes", get(read_processes))
        .route("/sessions/{id}/changes", get(read_changes))
        .route("/events", get(sse::events))
        .route("/commands", post(command))
        .route("/callbacks/{id}", post(answer_callback))
        .route("/server", get(server))
        .route("/ui", get(list_ui))
        .route("/ui/{*name}", post(ui_route))
        .merge(crate::terminal_routes::router())
        .merge(crate::window_routes::router())
        .fallback(not_found)
}

async fn not_found() -> Response {
    guard_error(StatusCode::NOT_FOUND, "notFound", "no such route")
}

/// Decodes a JSON body into `T`, refusing other content types. The refusal is boxed (an axum
/// `Response` is large and this is the cold path).
pub(crate) fn decode<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<T, Box<Response>> {
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"));
    if !json {
        return Err(Box::new(guard_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupportedMediaType",
            "send the body as application/json",
        )));
    }
    serde_json::from_slice(body)
        .map_err(|e| Box::new(ApiError(invalid_command(&e)).into_response()))
}

/// Runs a command for the Debug API origin.
pub(crate) async fn run(state: &ApiState, command: Command) -> Result<CommandResult, ApiError> {
    state
        .core
        .execute(Origin::DebugApi, command)
        .await
        .map_err(ApiError)
}

async fn health(State(state): State<ApiState>) -> Response {
    let status = state.core.status();
    ok(&json!({
        "ok": true,
        "apiVersion": crate::API_VERSION,
        "appVersion": status.app_version,
        "vibeVersion": status.vibe_version,
        "uptimeMs": status.uptime_ms,
        "enabledReason": state.reason.as_str(),
        "debugApi": status.debug_api,
        "rateLimits": status.rate_limits,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    limit: Option<u32>,
    cursor: Option<String>,
    #[serde(rename = "includeArchived")]
    include_archived: Option<bool>,
}

async fn list_sessions(
    State(state): State<ApiState>,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(ListQuery {
        limit,
        cursor,
        include_archived,
    })) = query
    else {
        return guard_error(
            StatusCode::BAD_REQUEST,
            "invalidCommand",
            "query parameters are `limit` (a number) and `cursor`",
        );
    };
    match run(
        &state,
        Command::ListSessions {
            limit,
            cursor,
            include_archived,
        },
    )
    .await
    {
        Ok(result) => ok(&result),
        Err(error) => error.into_response(),
    }
}

async fn read_session(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match run(&state, Command::ReadSession { session_id: id }).await {
        Ok(result) => ok(&result),
        Err(error) => error.into_response(),
    }
}

async fn read_anomalies(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match run(&state, Command::ReadAnomalies { session_id: id }).await {
        Ok(result) => ok(&result),
        Err(error) => error.into_response(),
    }
}

/// Read only: a fresh scan of the session's listeners (with owner and evidence), registered and re-attached
/// processes, Vibe background processes and leased ports. Another app's processes show name and pid only.
async fn read_processes(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match state.core.session_processes(&id).await {
        Ok(view) => ok(&view),
        Err(error) => ApiError(error).into_response(),
    }
}

/// The session's changes: git diff against its review baseline, or its turns' file edits in a
/// folder without git. Keep/revert go through `POST /commands` (`{"type": "changes",...}`).
async fn read_changes(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    let request = vw_core_api::ChangesRequest::Read { session_id: id };
    match run(&state, Command::Changes { request }).await {
        Ok(result) => ok(&result),
        Err(error) => error.into_response(),
    }
}

async fn command(State(state): State<ApiState>, headers: HeaderMap, body: Bytes) -> Response {
    let command: Command = match decode(&headers, &body) {
        Ok(command) => command,
        Err(response) => return *response,
    };
    match run(&state, command).await {
        Ok(result) => ok(&result),
        Err(error) => error.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CallbackAnswer {
    session_id: String,
    output: Value,
}

async fn answer_callback(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let answer: CallbackAnswer = match decode(&headers, &body) {
        Ok(answer) => answer,
        Err(response) => return *response,
    };
    let command = Command::AnswerCallback {
        session_id: answer.session_id,
        callback_id: id,
        output: answer.output,
    };
    match run(&state, command).await {
        Ok(result) => ok(&result),
        Err(error) => error.into_response(),
    }
}

async fn server(State(state): State<ApiState>) -> Response {
    ok(&state.core.status().server)
}

async fn list_ui(State(state): State<ApiState>) -> Response {
    let routes: Vec<Value> = state
        .ui_routes
        .list()
        .into_iter()
        .map(|(name, description)| json!({"name": name, "description": description}))
        .collect();
    ok(&json!({ "routes": routes }))
}

async fn ui_route(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(handler) = state.ui_routes.get(&name) else {
        return ApiError(CoreError::new(ErrorCode::NotFound, "no such ui route")).into_response();
    };
    let body: Value = match decode(&headers, &body) {
        Ok(body) => body,
        Err(response) => return *response,
    };
    match handler(body).await {
        Ok(value) => ok(&value),
        Err(error) => {
            state.core.note_failure(Origin::DebugApi, "ui", &error);
            ApiError(error).into_response()
        }
    }
}
