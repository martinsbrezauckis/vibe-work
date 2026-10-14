//! Terminal routes: thin REST adapters over `Command::Terminal`, so an external agent can
//! open, list, read, type into, resize, restart and close a session's terminals. Everything goes
//! through `Core::execute` with the Debug API origin, so typing needs the native confirmation exactly
//! as `POST /commands` with `{"type": "terminal", "request": {"op": "write",...}}` does.
//!
//! Depends on: axum, vw-core-api, `routes` (`run`, `decode`), `reply`. Called by: `routes` (merged).
//! Routes: `GET|POST /sessions/{id}/terminals`, `GET /terminals/{tid}/output?from&maxBytes&plain`,
//! `POST /terminals/{tid}/input`, `POST /terminals/{tid}/resize`, `POST /terminals/{tid}/restart`,
//! `DELETE /terminals/{tid}`.
//! Invariant: no route returns terminal input; reads return output only.

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde::Deserialize;
use vw_core_api::{Command, TerminalRequest};

use crate::reply::{guard_error, ok};
use crate::routes::{decode, run};
use crate::server::ApiState;

/// The terminal routes.
pub(crate) fn router() -> Router<ApiState> {
    Router::new()
        .route("/sessions/{id}/terminals", get(list).post(open))
        .route("/terminals/{tid}/output", get(output))
        .route("/terminals/{tid}/input", post(input))
        .route("/terminals/{tid}/resize", post(resize))
        .route("/terminals/{tid}/restart", post(restart))
        .route("/terminals/{tid}", delete(close))
}

async fn execute(state: &ApiState, request: TerminalRequest) -> Response {
    match run(state, Command::Terminal { request }).await {
        Ok(result) => ok(&result),
        Err(error) => error.into_response(),
    }
}

async fn list(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    execute(
        &state,
        TerminalRequest::List {
            session_id: Some(id),
        },
    )
    .await
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct OpenBody {
    cols: Option<u16>,
    rows: Option<u16>,
}

async fn open(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body: OpenBody = if body.is_empty() {
        OpenBody::default()
    } else {
        match decode(&headers, &body) {
            Ok(body) => body,
            Err(response) => return *response,
        }
    };
    execute(
        &state,
        TerminalRequest::Open {
            session_id: id,
            cols: body.cols,
            rows: body.rows,
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OutputQuery {
    from: Option<u64>,
    max_bytes: Option<u64>,
    plain: Option<bool>,
}

async fn output(
    State(state): State<ApiState>,
    Path(tid): Path<String>,
    query: Result<Query<OutputQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return guard_error(
            StatusCode::BAD_REQUEST,
            "invalidCommand",
            "query parameters are `from` and `maxBytes` (numbers) and `plain` (true/false)",
        );
    };
    execute(
        &state,
        TerminalRequest::Read {
            terminal_id: tid,
            from: query.from,
            max_bytes: query.max_bytes,
            plain: query.plain,
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputBody {
    data: String,
    binary: Option<bool>,
}

async fn input(
    State(state): State<ApiState>,
    Path(tid): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body: InputBody = match decode(&headers, &body) {
        Ok(body) => body,
        Err(response) => return *response,
    };
    execute(
        &state,
        TerminalRequest::Write {
            terminal_id: tid,
            data: body.data,
            binary: body.binary,
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeBody {
    cols: u16,
    rows: u16,
}

async fn resize(
    State(state): State<ApiState>,
    Path(tid): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body: ResizeBody = match decode(&headers, &body) {
        Ok(body) => body,
        Err(response) => return *response,
    };
    execute(
        &state,
        TerminalRequest::Resize {
            terminal_id: tid,
            cols: body.cols,
            rows: body.rows,
        },
    )
    .await
}

async fn restart(State(state): State<ApiState>, Path(tid): Path<String>) -> Response {
    execute(&state, TerminalRequest::Restart { terminal_id: tid }).await
}

async fn close(State(state): State<ApiState>, Path(tid): Path<String>) -> Response {
    execute(&state, TerminalRequest::Close { terminal_id: tid }).await
}
