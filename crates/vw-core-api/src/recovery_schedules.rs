//! Discover saved schedules once at app startup, independently of any window or read command.
//! Core starts this after deletion cleanup; inert listings wait for Vibe, then recovery rechecks each chat.
use std::sync::Arc;

use serde_json::{Value, json};

use crate::core::Shared;

pub(crate) fn start(shared: &Arc<Shared>) {
    let weak = Arc::downgrade(shared);
    let mut states = shared.supervisor.state();
    tokio::spawn(async move {
        while !matches!(
            *states.borrow_and_update(),
            vw_supervisor::SupervisorState::Ready { .. }
        ) {
            if states.changed().await.is_err() {
                return;
            }
        }
        let Some(shared) = weak.upgrade() else { return };
        let mut cursor = Value::Null;
        loop {
            let request = shared.gate.call(
                "session/list",
                json!({"limit":100,"includeArchived":false,"cursor":cursor}),
                None,
            );
            let reply = match tokio::time::timeout(shared.config.reopen_timeout, request).await {
                Ok(Ok(reply)) => reply,
                result => {
                    tracing::warn!(?result, "could not discover saved schedules at startup");
                    return;
                }
            };
            let scheduled = reply.result["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|session| {
                    session["archivedAt"].is_null()
                        && session["loops"]
                            .as_array()
                            .is_some_and(|loops| !loops.is_empty())
                })
                .filter_map(|session| session["id"].as_str().map(|id| (id.to_owned(), Vec::new())))
                .collect();
            crate::recovery::restore_detached(&shared, scheduled);
            let next = &reply.result["nextCursor"];
            if next.is_null() || next == &cursor {
                return;
            }
            cursor = next.clone();
        }
    });
}
