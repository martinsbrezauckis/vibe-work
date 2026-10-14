//! The app window through the Debug API: whole-window screenshots as PNG, the list of the app's own
//! windows (the main window and native dialogs that belong to Vibe Work), and the window's size and state.
//!
//! "getting back evidence as app screenshot through debug was very useful in dev process for
//! ShellX"; scope: only the app's own windows, never the screen or another app.
//!
//! Routes: `GET /windows`, `GET /screenshot?window=<id>&crop=<css selector>&pad=<px>`, `POST /window`
//! (`{"action": "state" | "size" | "maximize" | "restore" | "minimize" | "focus", "width"?, "height"?}`).
//!
//! Depends on: axum, vw-preview (`decode`, `crop`, `encode`), `registry` (the UI's `rect` route for crops),
//! `server::ApiState`. Called by: `routes` (merged into the router). The app supplies the [`AppWindow`]
//! (Tauri; vw-webview captures); a run without a window (test hosts, the browser dev UI) answers 503 `noWindow`.
//! Invariants:
//! - A screenshot is of one of the app's own windows, captured from that window (Windows: `PrintWindow`;
//!   Linux, macOS: the window's own webviews composed), never from the screen.
//! - A crop is the element's bounding rectangle the UI reports (CSS pixels, scaled to the capture's pixels),
//!   clipped to the image; an unknown selector is `404 notFound`, nothing is guessed.
//! - Secret fields are blanked (the secret rule): before a main-window PNG leaves, the UI's `secrets`
//!   route names the rectangles of every secret field shown (key and password fields) and they are painted over
//!   with a flat grey; the answer says how many (`x-vw-secret-blanked`). The UI's capture lock holds across the
//!   capture (secret fields show dots, Show is disabled) and the answers before and after must match, else the
//!   image is discarded (`main_capture`; commit review "toctou"). When the UI cannot answer, the screenshot is
//!   refused (`503 secretCheckFailed`): fail closed, never a capture that might show a secret.
//! - A non-main id is captured only when `GET /windows` lists it as a dialog right now ([`listed_dialog`];
//!   security rule): never the main window under another id (its pixels go out only through the blanking path
//!   above) and never an unlisted helper window. The app's capture checks the same again (vw-webview).
//! - PNG answers are `image/png` with `Cache-Control: no-store`; they are not text, so the secret guard does not
//!   search them (`secret_guard.rs`).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::reply::{guard_error, ok};
use crate::routes::decode;
use crate::server::ApiState;

/// A boxed future, so [`AppWindow`] stays object safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One of the app's own windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowView {
    /// Id for `/screenshot?window=` (`main`, or a dialog's id).
    pub id: String,
    /// `main` or `dialog`.
    pub kind: String,
    /// The window title.
    pub title: String,
    /// Size in physical pixels.
    pub width: u32,
    /// Size in physical pixels.
    pub height: u32,
}

/// What `POST /window` asks.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum WindowRequest {
    /// Read the state.
    State {},
    /// Set the inner size in logical (CSS) pixels; the OS may clamp it to the minimum size.
    Size {
        /// Width.
        width: f64,
        /// Height.
        height: f64,
    },
    /// Maximize.
    Maximize {},
    /// Leave maximized or minimized.
    Restore {},
    /// Minimize.
    Minimize {},
    /// Bring the window to the front and focus it.
    Focus {},
}

/// The main window's state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    /// Inner width in logical pixels.
    pub width: f64,
    /// Inner height in logical pixels.
    pub height: f64,
    /// Device pixels per logical pixel.
    pub scale: f64,
    /// Maximized.
    pub maximized: bool,
    /// Minimized.
    pub minimized: bool,
    /// Has the keyboard focus.
    pub focused: bool,
    /// The configured minimum inner size, logical pixels.
    pub min_width: f64,
    /// The configured minimum inner size, logical pixels.
    pub min_height: f64,
}

/// The app's windows as the Debug API sees them (implemented by the app on Tauri).
pub trait AppWindow: Send + Sync {
    /// The app's own windows: the main window first, then native dialogs the OS lets the app see.
    fn windows(&self) -> BoxFuture<'_, Result<Vec<WindowView>, String>>;
    /// A PNG of window `id` (sRGB).
    fn capture(&self, id: String) -> BoxFuture<'_, Result<Vec<u8>, String>>;
    /// Acts on the main window and returns its state afterwards.
    fn control(&self, request: WindowRequest) -> BoxFuture<'_, Result<WindowState, String>>;

    /// Test builds only (feature `e2e-script`): evaluates `js` (already wrapped by the route) in the main
    /// webview and returns the engine's JSON result text; [`crate::e2e_script::TIMEOUT`] when the page did not
    /// answer in time. The default refuses, so a host without an engine (test hosts) needs no implementation.
    #[cfg(feature = "e2e-script")]
    fn script(&self, js: String) -> BoxFuture<'_, Result<String, String>> {
        let _ = js;
        Box::pin(async { Err("this window cannot run scripts".to_owned()) })
    }

    /// Test builds only (feature `e2e-script`): the labels of the app's webview windows (the `WebDriver`
    /// window handles). The default refuses.
    #[cfg(feature = "e2e-script")]
    fn e2e_windows(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async { Err("this window cannot list webview windows".to_owned()) })
    }
}

/// The shared handle the app passes in.
pub type SharedAppWindow = Arc<dyn AppWindow>;

pub(crate) fn router() -> Router<ApiState> {
    Router::new()
        .route("/windows", get(windows))
        .route("/screenshot", get(screenshot))
        .route("/window", post(window))
}

fn no_window() -> Response {
    guard_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "noWindow",
        "this run has no app window (a test host or the browser dev UI)",
    )
}

fn failed(message: &str) -> Response {
    guard_error(StatusCode::SERVICE_UNAVAILABLE, "windowFailed", message)
}

/// True when `id` is a dialog the current listing names (never `main`: that one goes through `main_capture`).
#[must_use]
pub fn listed_dialog(listing: &[WindowView], id: &str) -> bool {
    id != "main" && listing.iter().any(|w| w.id == id && w.kind == "dialog")
}

async fn windows(State(state): State<ApiState>) -> Response {
    let Some(app) = state.window.clone() else {
        return no_window();
    };
    match app.windows().await {
        Ok(list) => ok(&json!({ "windows": list })),
        Err(message) => failed(&message),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShotQuery {
    window: Option<String>,
    crop: Option<String>,
    pad: Option<u32>,
}

/// Asks the UI where `selector` is (the `rect` UI route) and returns it in the capture's pixels.
async fn element_rect(
    state: &ApiState,
    selector: &str,
    pad: u32,
    image_width: u32,
) -> Result<vw_preview::PixelRect, Box<Response>> {
    let Some(handler) = state.ui_routes.get("rect") else {
        return Err(Box::new(guard_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "noWindow",
            "the UI does not answer element rectangles in this run",
        )));
    };
    let answer = handler(json!({ "selector": selector }))
        .await
        .map_err(|e| Box::new(crate::reply::ApiError(e).into_response()))?;
    if answer.get("found").and_then(Value::as_bool) != Some(true) {
        return Err(Box::new(guard_error(
            StatusCode::NOT_FOUND,
            "notFound",
            "no element matches that selector",
        )));
    }
    let num = |key: &str| answer.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let viewport = num("viewportWidth").max(1.0);
    let scale = f64::from(image_width) / viewport;
    let pad = f64::from(pad);
    // CSS pixels times the capture scale, rounded: window sizes fit an i64 by far.
    #[allow(clippy::cast_possible_truncation)]
    let px = |v: f64| {
        let rounded = (v * scale).round();
        if rounded.is_finite() {
            rounded as i64
        } else {
            0
        }
    };
    let size = |v: f64| u32::try_from(px(v).max(0)).unwrap_or(0);
    Ok(vw_preview::PixelRect {
        x: px(num("x") - pad),
        y: px(num("y") - pad),
        width: size(num("width") + 2.0 * pad),
        height: size(num("height") + 2.0 * pad),
    })
}

/// What the UI's `secrets` route answered: the secret fields' rectangles (CSS pixels), how many show their value
/// in clear, the capture lock's epoch, the viewport width.
#[derive(Debug, PartialEq)]
struct SecretState {
    rects: Vec<[f64; 4]>,
    clear: u64,
    epoch: u64,
    viewport: f64,
}

/// Asks the UI where secret fields are, turning the capture lock on (`lock = true`, before a capture) or off
/// (`false`, after it; the answer is read before the lock is released).
async fn secret_state(state: &ApiState, lock: bool) -> Result<SecretState, Box<Response>> {
    let refused = |why: &str| {
        Box::new(guard_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "secretCheckFailed",
            &format!(
                "the screenshot is refused: the UI could not say where secret fields are ({why})"
            ),
        ))
    };
    let handler = state
        .ui_routes
        .get("secrets")
        .ok_or_else(|| refused("no secrets route"))?;
    let answer = handler(json!({ "lock": lock }))
        .await
        .map_err(|e| refused(&e.message))?;
    let viewport = answer
        .get("viewportWidth")
        .and_then(Value::as_f64)
        .filter(|w| *w >= 1.0)
        .ok_or_else(|| refused("no viewport width"))?;
    let rects = answer
        .get("rects")
        .and_then(Value::as_array)
        .ok_or_else(|| refused("no rectangles"))?
        .iter()
        .map(|r| {
            let num = |k: &str| r.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            [num("x"), num("y"), num("width"), num("height")]
        })
        .collect();
    Ok(SecretState {
        rects,
        clear: answer.get("clear").and_then(Value::as_u64).unwrap_or(0),
        epoch: answer.get("epoch").and_then(Value::as_u64).unwrap_or(0),
        viewport,
    })
}

/// `rects` (CSS pixels) in the capture's pixels for an image `image_width` wide, one pixel larger on every side.
fn to_pixels(found: &SecretState, image_width: u32) -> Vec<vw_preview::PixelRect> {
    let scale = f64::from(image_width) / found.viewport;
    // CSS pixels times the capture scale, rounded: window sizes fit an i64 by far.
    #[allow(clippy::cast_possible_truncation)]
    let px = |v: f64| (v * scale).round() as i64;
    found
        .rects
        .iter()
        .map(|[x, y, w, h]| vw_preview::PixelRect {
            x: px(*x) - 1,
            y: px(*y) - 1,
            width: u32::try_from(px(*w).max(0) + 2).unwrap_or(u32::MAX),
            height: u32::try_from(px(*h).max(0) + 2).unwrap_or(u32::MAX),
        })
        .collect()
}

/// The main window captured under the capture lock with every secret field blanked, and how many were blanked.
/// The lock holds across the capture; the answers before and after must match (same rectangles, same number
/// shown in clear, same lock epoch), else the image is discarded and the capture tried once more, then refused
/// (`409 secretMoved`). One screenshot at a time (`ApiState::shots`), so two cannot release each other's lock.
async fn main_capture(
    state: &ApiState,
    app: &SharedAppWindow,
) -> Result<(Vec<u8>, usize), Box<Response>> {
    let _one_at_a_time = state.shots.lock().await;
    for _ in 0..2 {
        let before = secret_state(state, true).await?;
        let captured = app.capture("main".to_owned()).await;
        let after = secret_state(state, false).await?;
        let png = captured.map_err(|m| Box::new(failed(&m)))?;
        if before != after {
            tracing::warn!("window screenshot discarded: secret fields changed during the capture");
            continue;
        }
        let mut image = vw_preview::decode(&png).map_err(|m| Box::new(failed(&m)))?;
        let rects = to_pixels(&after, image.width);
        for rect in &rects {
            let patch = vw_preview::canvas(rect.width, rect.height, [128, 128, 128, 255]);
            vw_preview::paste(&mut image, &patch, *rect);
        }
        let png = vw_preview::encode(&image).map_err(|m| Box::new(failed(&m)))?;
        return Ok((png, rects.len()));
    }
    Err(Box::new(guard_error(
        StatusCode::CONFLICT,
        "secretMoved",
        "the screenshot is refused: secret fields changed while it was taken (twice)",
    )))
}

async fn screenshot(
    State(state): State<ApiState>,
    query: Result<Query<ShotQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(ShotQuery { window, crop, pad })) = query else {
        return guard_error(
            StatusCode::BAD_REQUEST,
            "invalidCommand",
            "query parameters are `window`, `crop` (a CSS selector) and `pad` (pixels)",
        );
    };
    let Some(app) = state.window.clone() else {
        return no_window();
    };
    let id = window.unwrap_or_else(|| "main".to_owned());
    let (mut png, blanked) = if id == "main" {
        match main_capture(&state, &app).await {
            Ok(shot) => shot,
            Err(response) => return *response,
        }
    } else {
        match app.windows().await {
            Ok(listing) if listed_dialog(&listing, &id) => {}
            Ok(_) => {
                return guard_error(
                    StatusCode::NOT_FOUND,
                    "notFound",
                    "no such dialog (it may have closed); GET /windows lists the windows",
                );
            }
            Err(message) => return failed(&message),
        }
        match app.capture(id.clone()).await {
            Ok(png) => (png, 0),
            Err(message) => return failed(&message),
        }
    };
    if let Some(selector) = crop.filter(|_| id == "main") {
        let image = match vw_preview::decode(&png) {
            Ok(image) => image,
            Err(message) => return failed(&message),
        };
        let rect = match element_rect(&state, &selector, pad.unwrap_or(0), image.width).await {
            Ok(rect) => rect,
            Err(response) => return *response,
        };
        let Some(part) = vw_preview::crop(&image, rect) else {
            return guard_error(
                StatusCode::NOT_FOUND,
                "notFound",
                "the element is outside the window",
            );
        };
        png = match vw_preview::encode(&part) {
            Ok(png) => png,
            Err(message) => return failed(&message),
        };
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png".to_owned()),
            (header::CACHE_CONTROL, "no-store".to_owned()),
            (
                header::HeaderName::from_static("x-vw-secret-blanked"),
                blanked.to_string(),
            ),
        ],
        png,
    )
        .into_response()
}

async fn window(State(state): State<ApiState>, headers: HeaderMap, body: Bytes) -> Response {
    let request: WindowRequest = match decode(&headers, &body) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    let Some(app) = state.window.clone() else {
        return no_window();
    };
    match app.control(request).await {
        Ok(view) => ok(&view),
        Err(message) => failed(&message),
    }
}
