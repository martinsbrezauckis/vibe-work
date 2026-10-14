//! Test builds only: the in-page channel the end-to-end runs use where no WebDriver exists (macOS:
//! WKWebView has none). `POST /e2e/script` evaluates a script in the main webview and answers its JSON result;
//! `GET /e2e/windows` lists the app's webview windows (what WebDriver calls window handles). The private e2e
//! suite's direct driver builds the WebDriver subset its checks use on top of these two routes, `GET /screenshot`
//! and `POST /window`.
//!
//! Compiled only with the cargo feature `e2e-script`, which refuses to build without `debug_assertions` (see
//! `lib.rs`): no release build has these routes.
//!
//! Depends on: axum, `window_routes::AppWindow` (the app evaluates the script; Tauri `eval_with_callback`), `reply`,
//! `routes::decode`. Called by: `routes` (merged into the router, inside the guard and the secret guard).
//! Invariants:
//! - Same rules as every route: bearer token, loopback, Host and Origin checks (the router's guard layer), and
//!   every answer passes the secret guard (registered secret values are replaced; header `x-vw-secret-withheld`).
//! - The evaluated text is always `WRAP_HEAD + script + WRAP_TAIL`: the page returns one JSON string
//!   (`{"ok":true,"value":...}` or `{"ok":false,"error":"..."}`) whatever the script does, because a native
//!   object that cannot become JSON would abort the engine's completion handler (WKWebView serializes the result
//!   with `NSJSONSerialization`, which raises on such values).
//! - The script text is never logged (it may hold text a test types into a field); only its size is.

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::reply::{guard_error, ok};
use crate::routes::decode;
use crate::server::ApiState;

/// The longest script accepted (bytes). The test client sends its helper library with every call (~20 KB).
pub const MAX_SCRIPT_BYTES: usize = 1024 * 1024;

/// Opens the wrapper: the script becomes the body of a function, as in the `WebDriver` `execute/sync` command.
pub const WRAP_HEAD: &str = "(function(){try{var vwE2eResult=(function(){\n";

/// Closes the wrapper: the result (or the error) goes back as one JSON string.
pub const WRAP_TAIL: &str = "\n})();return JSON.stringify({ok:true,value:vwE2eResult===undefined?null:vwE2eResult});}\
catch(e){try{return JSON.stringify({ok:false,error:String(e&&e.stack?e.stack:e)});}\
catch(f){return '{\"ok\":false,\"error\":\"the error could not be encoded\"}';}}})()";

/// The text the webview evaluates for `script`.
#[must_use]
pub fn wrap(script: &str) -> String {
    let mut out = String::with_capacity(WRAP_HEAD.len() + script.len() + WRAP_TAIL.len());
    out.push_str(WRAP_HEAD);
    out.push_str(script);
    out.push_str(WRAP_TAIL);
    out
}

/// What the page answered, decoded from the engine's result text.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptOutcome {
    /// The script returned this value (`null` for `undefined`).
    Value(Value),
    /// The script threw; the text is the error's stack or message.
    Threw(String),
    /// The engine gave no result: a syntax error in the script, or the page was not loaded.
    NoResult,
}

/// Decodes the engine's result text: the JSON of the string the wrapper returned (`WebKitGTK` and `WKWebView` give the
/// JSON of the returned value, `WebView2` likewise), which itself is the JSON of `{ok, value | error}`.
#[must_use]
pub fn decode_result(raw: &str) -> ScriptOutcome {
    let Ok(Value::String(inner)) = serde_json::from_str::<Value>(raw.trim()) else {
        return ScriptOutcome::NoResult;
    };
    let Ok(Value::Object(mut body)) = serde_json::from_str::<Value>(&inner) else {
        return ScriptOutcome::NoResult;
    };
    if body.get("ok").and_then(Value::as_bool) == Some(true) {
        ScriptOutcome::Value(body.remove("value").unwrap_or(Value::Null))
    } else {
        ScriptOutcome::Threw(
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_owned(),
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptRequest {
    script: String,
}

pub(crate) fn router() -> Router<ApiState> {
    Router::new()
        .route("/e2e/script", post(script))
        .route("/e2e/windows", get(windows))
}

fn no_window() -> Response {
    guard_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "noWindow",
        "this run has no app window (a test host or the browser dev UI)",
    )
}

async fn script(State(state): State<ApiState>, headers: HeaderMap, body: Bytes) -> Response {
    let request: ScriptRequest = match decode(&headers, &body) {
        Ok(r) => r,
        Err(refusal) => return *refusal,
    };
    if request.script.len() > MAX_SCRIPT_BYTES {
        return guard_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "scriptTooLarge",
            "the script is longer than 1 MiB",
        );
    }
    let Some(app) = state.window.clone() else {
        return no_window();
    };
    tracing::debug!(bytes = request.script.len(), "e2e script");
    match app.script(wrap(&request.script)).await {
        Ok(raw) => match decode_result(&raw) {
            ScriptOutcome::Value(value) => ok(&json!({ "value": value })),
            ScriptOutcome::Threw(error) => {
                guard_error(StatusCode::UNPROCESSABLE_ENTITY, "scriptError", &error)
            }
            ScriptOutcome::NoResult => guard_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "scriptError",
                "the script gave no result (a syntax error, or the page is not loaded yet)",
            ),
        },
        Err(message) if message == TIMEOUT => {
            guard_error(StatusCode::GATEWAY_TIMEOUT, "scriptTimeout", &message)
        }
        Err(message) => guard_error(StatusCode::SERVICE_UNAVAILABLE, "windowFailed", &message),
    }
}

/// The error text an [`crate::AppWindow::script`] implementation returns when the engine did not answer in time;
/// the route answers it as `504 scriptTimeout`.
pub const TIMEOUT: &str = "the page did not answer the script within 30 s";

async fn windows(State(state): State<ApiState>) -> Response {
    let Some(app) = state.window.clone() else {
        return no_window();
    };
    match app.e2e_windows().await {
        Ok(labels) => ok(&json!({ "windows": labels })),
        Err(message) => guard_error(StatusCode::SERVICE_UNAVAILABLE, "windowFailed", &message),
    }
}
