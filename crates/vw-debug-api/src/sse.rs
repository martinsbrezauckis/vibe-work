//! `GET /events`: the core's event stream as Server-Sent Events, for all sessions or one
//! (`?session=<id>`).
//!
//! Depends on: axum (SSE), `futures-util` (stream), vw-core-api (`CoreSubscription`),
//! `server::ApiState`. Called by: `routes`.
//! Invariants:
//! - First the snapshots, then ordered changes and core events (the subscription's rules); a
//!   lagging client gets a `lagged` event and fresh snapshots, never a silent hole.
//! - Each SSE event has `event: <type>` (`snapshot`, `change`, `lagged`, `server`,...), a
//!   per-stream increasing `id`, and the `CoreEvent` JSON as `data`, every stored secret value in it
//!   replaced by `[secret withheld: <kind>]`.
//! - The stream ends when the server shuts down (graceful shutdown would otherwise wait for
//!   every open stream) or when the core is gone.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use tokio::sync::watch;
use vw_core_api::{CoreEvent, CoreSubscription};

use crate::reply::guard_error;
use crate::server::ApiState;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventsQuery {
    session: Option<String>,
}

fn event_name(event: &CoreEvent) -> &'static str {
    match event {
        CoreEvent::Snapshot { .. } => "snapshot",
        CoreEvent::Change { .. } => "change",
        CoreEvent::Lagged {} => "lagged",
        CoreEvent::Server { .. } => "server",
        CoreEvent::WorktreesChanged {} => "worktreesChanged",
        CoreEvent::CallbackOpened { .. } => "callbackOpened",
        CoreEvent::CallbackAnswered { .. } => "callbackAnswered",
        CoreEvent::Refused { .. } => "refused",
        CoreEvent::Alert { .. } => "alert",
        CoreEvent::SessionStuck { .. } => "sessionStuck",
        CoreEvent::SessionHolder { .. } => "sessionHolder",
        CoreEvent::DebugApi { .. } => "debugApi",
        CoreEvent::KeyNeeded { .. } => "keyNeeded",
        CoreEvent::KeyStatusChanged {} => "keyStatusChanged",
        CoreEvent::VibeInstall { .. } => "vibeInstall",
        CoreEvent::RateLimits { .. } => "rateLimits",
        CoreEvent::Terminal { .. } => "terminal",
        CoreEvent::DocumentsAdded { .. } => "documentsAdded",
        CoreEvent::DocumentChanged { .. } => "documentChanged",
        CoreEvent::ChangesUpdated { .. } => "changesUpdated",
        CoreEvent::PreviewChanged { .. } => "previewChanged",
        CoreEvent::PreviewShowRequested { .. } => "previewShowRequested",
        CoreEvent::BrowserChanged { .. } => "browserChanged",
        CoreEvent::PagesChanged { .. } => "pagesChanged",
        CoreEvent::FilesChanged { .. } => "filesChanged",
        CoreEvent::LineCommentsChanged { .. } => "lineCommentsChanged",
        CoreEvent::ProjectsChanged {} => "projectsChanged",
        CoreEvent::SecretsChanged {} => "secretsChanged",
        CoreEvent::Dictation { .. } => "dictation",
        CoreEvent::ConnectorSignIn { .. } => "connectorSignIn",
        CoreEvent::MarketInstall { .. } => "marketInstall",
    }
}

struct Feed {
    sub: CoreSubscription,
    stop: watch::Receiver<bool>,
    next_id: u64,
}

/// The SSE item for one core event (an encoding failure becomes a comment line, never a panic). Every stored
/// secret value in it is replaced by its marker first (`secret_guard`).
fn sse_event(event: &CoreEvent, id: u64) -> Event {
    let Ok(json) = serde_json::to_string(event) else {
        return Event::default().comment("an event could not be encoded");
    };
    Event::default()
        .event(event_name(event))
        .id(id.to_string())
        .data(crate::secret_guard::scrub_event(json))
}

fn feed(
    sub: CoreSubscription,
    stop: watch::Receiver<bool>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let state = Feed {
        sub,
        stop,
        next_id: 1,
    };
    stream::unfold(state, |mut state| async move {
        if *state.stop.borrow() {
            return None;
        }
        let event = tokio::select! {
            event = state.sub.next() => event?,
            _ = state.stop.wait_for(|stop| *stop) => return None,
        };
        let id = state.next_id;
        state.next_id += 1;
        Some((Ok(sse_event(&event, id)), state))
    })
}

pub(crate) async fn events(
    State(state): State<ApiState>,
    query: Result<Query<EventsQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(EventsQuery { session })) = query else {
        return guard_error(
            StatusCode::BAD_REQUEST,
            "invalidCommand",
            "the only query parameter is `session`",
        );
    };
    let sub = state.core.subscribe(session);
    let keep_alive = KeepAlive::new().interval(state.keep_alive.max(Duration::from_secs(1)));
    Sse::new(feed(sub, state.stop.clone()))
        .keep_alive(keep_alive)
        .into_response()
}
