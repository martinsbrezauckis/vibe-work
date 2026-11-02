//! macOS: the agent browser's own script world and the private request watch, as async calls on a Tauri webview.
//!
//! - [`install_world`] puts the agent world into a preview webview (script in every frame, cross-site frames
//!   included; on macOS 27 closed shadow roots read as open there). [`evaluate_in_frames`] and
//!   [`evaluate_main`] run a script in that world; [`evaluate_in_child_frames_by_handle`] uses macOS 27's JS
//!   handles to find child frames.
//! - [`watch_requests`] attaches Apple's PRIVATE watch-only request delegate, only while [`set_request_watch`]
//!   is on (Settings > Developer; off by default). A macOS without the private interface leaves it off quietly.
//!
//! - [`show_click_marker`] draws a short ring where the agent clicked, in the person's view only (never the page).
//!
//! Depends on: vw-platform `webview::{agent_world_mac, request_watch_mac, click_marker_mac}`, `platform::with_native`. Called by:
//! the engine and its private test fixture.
//! Invariant: nothing here installs page-world script or reaches the page's own JavaScript.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{Webview, Wry};
use vw_preview::EngineError;

pub use vw_platform::webview::agent_world_mac::{FrameAnswer, WorldFacts};
pub use vw_platform::webview::request_watch_mac::{RequestEvent, RequestSink, RequestStep};

use crate::platform::with_native;

/// Longest wait for a script answer in the agent world.
const SCRIPT_WAIT: Duration = Duration::from_secs(10);

/// Settings > Developer: watch requests through the private interface (off by default).
static REQUEST_WATCH: AtomicBool = AtomicBool::new(false);

/// Turns the private request watch on or off for previews attached from now on.
pub fn set_request_watch(on: bool) {
    REQUEST_WATCH.store(on, Ordering::SeqCst);
}

/// Whether the private request watch is switched on.
#[must_use]
pub fn request_watch_on() -> bool {
    REQUEST_WATCH.load(Ordering::SeqCst)
}

/// Installs the agent world on `webview` (idempotent) and answers what it can do on this macOS.
///
/// # Errors
/// The webview is gone or did not answer.
pub async fn install_world(webview: &Webview<Wry>) -> Result<WorldFacts, EngineError> {
    with_native(
        webview,
        "installing the agent world",
        Duration::from_secs(5),
        |native, reply| {
            let _ = reply.send(vw_platform::webview::agent_world_mac::install_agent_world(
                native.inner(),
            ));
        },
    )
    .await
}

/// Runs `script` in the agent world of every frame of the current document.
///
/// # Errors
/// The world is not installed, the webview is gone, or no answer in time.
pub async fn evaluate_in_frames(
    webview: &Webview<Wry>,
    script: &str,
) -> Result<Vec<FrameAnswer>, EngineError> {
    let script = script.to_owned();
    with_native(
        webview,
        "running a script in every frame",
        SCRIPT_WAIT,
        move |native, reply| {
            vw_platform::webview::agent_world_mac::evaluate_in_frames(
                native.inner(),
                &script,
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await
}

/// Runs `script` in the main frame, in the agent world or (`page_world`) the page's own world.
///
/// # Errors
/// As [`evaluate_in_frames`]; a script error comes back as `Ok(Err(text))`.
pub async fn evaluate_main(
    webview: &Webview<Wry>,
    script: &str,
    page_world: bool,
) -> Result<Result<String, String>, EngineError> {
    let script = script.to_owned();
    with_native(
        webview,
        "running a script",
        SCRIPT_WAIT,
        move |native, reply| {
            vw_platform::webview::agent_world_mac::evaluate_main(
                native.inner(),
                &script,
                page_world,
                Box::new(move |result| {
                    let _ = reply.send(Ok(result));
                }),
            );
        },
    )
    .await
}

/// macOS 27: runs `script` in each direct child frame, found through JS handles.
///
/// # Errors
/// Before macOS 27, the world is not installed, or no answer in time.
pub async fn evaluate_in_child_frames_by_handle(
    webview: &Webview<Wry>,
    script: &str,
) -> Result<Vec<FrameAnswer>, EngineError> {
    let script = script.to_owned();
    with_native(
        webview,
        "running a script in child frames",
        SCRIPT_WAIT,
        move |native, reply| {
            vw_platform::webview::agent_world_mac::evaluate_in_child_frames_by_handle(
                native.inner(),
                &script,
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await
}

/// Attaches the private request watch when the Developer switch is on. Answers whether it is watching: `false`
/// when switched off or when this macOS no longer has the private interface (quietly, no notice).
///
/// # Errors
/// The webview is gone.
pub async fn watch_requests(
    webview: &Webview<Wry>,
    sink: RequestSink,
) -> Result<bool, EngineError> {
    if !request_watch_on() {
        return Ok(false);
    }
    with_native(
        webview,
        "watching requests",
        Duration::from_secs(5),
        move |native, reply| {
            let _ = reply.send(Ok(vw_platform::webview::request_watch_mac::attach(
                native.inner(),
                sink,
            )));
        },
    )
    .await
}

/// Stops the request watch and forgets the agent world of a webview that closes.
///
/// # Errors
/// The webview is gone.
pub async fn forget(webview: &Webview<Wry>) -> Result<(), EngineError> {
    with_native(
        webview,
        "forgetting the agent world",
        Duration::from_secs(5),
        |native, reply| {
            vw_platform::webview::request_watch_mac::detach(native.inner());
            vw_platform::webview::agent_world_mac::forget_agent_world(native.inner());
            let _ = reply.send(Ok(()));
        },
    )
    .await
}

/// Shows the agent's click marker at `point` (0..1 of the view, from the top left) for a moment.
///
/// # Errors
/// The webview is gone or the point is outside it.
pub async fn show_click_marker(
    webview: &Webview<Wry>,
    point: (f64, f64),
) -> Result<(), EngineError> {
    with_native(
        webview,
        "showing the click marker",
        Duration::from_secs(2),
        move |native, reply| {
            let _ = reply.send(vw_platform::webview::click_marker_mac::show(
                native.inner(),
                point,
            ));
        },
    )
    .await
}

/// The marker shown now, as (x, y, width, height) in the view, and whether a click at its centre still reaches the
/// page (tests).
///
/// # Errors
/// The webview is gone.
pub async fn click_marker_state(
    webview: &Webview<Wry>,
) -> Result<(Option<(f64, f64, f64, f64)>, bool), EngineError> {
    with_native(
        webview,
        "reading the click marker",
        Duration::from_secs(2),
        |native, reply| {
            let _ = reply.send(Ok((
                vw_platform::webview::click_marker_mac::shown(native.inner()),
                vw_platform::webview::click_marker_mac::passes_input(native.inner()),
            )));
        },
    )
    .await
}

#[cfg(test)]
mod detached_tests {
    /// The registry drops a removed frame's handle by the exact answer the preview's detached-frame check gives.
    #[test]
    fn the_registry_knows_the_preview_detached_answer() {
        assert_eq!(
            vw_platform::webview::agent_world_mac::DETACHED_ANSWER,
            vw_preview::DETACHED_FRAME
        );
    }
}
