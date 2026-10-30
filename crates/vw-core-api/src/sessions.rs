//! Store bookkeeping around forwarded calls: results that carry a session state are loaded into
//! the store with the right merge rule, new sessions are tracked for restarts, deleted ones
//! dropped. Applies to typed commands and to the generic `call` alike, so the store can never
//! miss a session that a client created.
//!
//! Depends on: vw-session (`LoadKind`, store), vw-supervisor (tracking), `session_end` (delete), `handoff`
//! (a result that moved the conversation to a new id), `core::Shared`, `error`, `view`. Called by:
//! `dispatch` after every successful gate call.
//! Invariants:
//! - Which methods load what (vw-session `LoadKind` docs): `session/start` and `session/fork`
//!   load a new session; `session/resume` and `session/continue` a resumed one; `session/rewind`,
//!   `session/compact` and `session/history/clear` replace the state (a rewind that is not in
//!   place yields a new session id, which is loaded as new); `session/read` merges a page.
//! - The store update is handed to the pump (bounded channel) for callbacks and resyncs.

use std::sync::Arc;

use serde_json::Value;
use vw_session::LoadKind;

use crate::core::Shared;
use crate::error::CoreError;
use crate::view::SessionSnapshot;

/// How a method's result touches the store.
enum Effect {
    Load(LoadKind),
    Delete,
    Archive,
    None,
}

fn effect(method: &str) -> Effect {
    match method {
        "session/start" | "session/fork" => Effect::Load(LoadKind::New),
        "session/resume" | "session/continue" => Effect::Load(LoadKind::Resume),
        "session/rewind" | "session/compact" | "session/history/clear" => {
            Effect::Load(LoadKind::Replace)
        }
        "session/read" => Effect::Load(LoadKind::Read),
        "session/delete" => Effect::Delete,
        "session/archive" => Effect::Archive,
        _ => Effect::None,
    }
}

/// Applies the store effect of a successful `method` call. Returns the id of the session whose
/// state was loaded, when one was.
///
/// # Errors
/// [`CoreError`] (`internal`) when a loading method's result carries no usable state.
pub(crate) async fn observe(
    shared: &Arc<Shared>,
    method: &str,
    params: &Value,
    result: &Value,
) -> Result<Option<String>, CoreError> {
    // The profile each session runs with (`access.rs`): from any result that carries a runtime, and
    // from an accepted switch.
    if let Some(id) = params.get("sessionId").and_then(Value::as_str) {
        if method == "session/agent/update" {
            shared.access.observe_request(method, id, params);
        }
        // Only an attached, verified runtime can replace the record.
        if let Some(runtime) = result
            .get("runtime")
            .filter(|_| shared.channels.has_channel(id))
        {
            shared.access.observe_runtime(id, runtime);
        }
    }
    shared.model_choices.observe(method, params, result)?;
    if method == "turn/start" {
        crate::model_choices::turn_started(shared, params).await;
    }
    match effect(method) {
        Effect::None => Ok(None),
        Effect::Archive => {
            crate::session_end::archive_decision(shared, params).await;
            Ok(None)
        }
        Effect::Delete => {
            if let Some(id) = params.get("sessionId").and_then(Value::as_str) {
                // Everything of a deleted session ends in one place (`session_end.rs`).
                crate::session_end::deleted(shared, id).await;
            }
            Ok(None)
        }
        Effect::Load(kind) => {
            let state = result
                .get("state")
                .filter(|s| s.is_object())
                .cloned()
                .ok_or_else(|| CoreError::bad_result(method, "state"))?;
            let id = state
                .pointer("/session/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| CoreError::bad_result(method, "state.session.id"))?;
            // A rewind or compaction that produced another session is a new session for the
            // store (local-only until listed), not a replacement of the source.
            let source = params.get("sessionId").and_then(Value::as_str);
            let kind = match kind {
                LoadKind::Replace if source.is_some_and(|s| s != id) => LoadKind::New,
                other => other,
            };
            if kind == LoadKind::Resume {
                crate::worktrees_restore::resumed(shared, &state).await?;
            }
            if kind != LoadKind::Read {
                let previous = shared.store.view(&id);
                if let Err(reason) = crate::session_folders::check(
                    &shared.instructions.own_home,
                    &state,
                    previous.as_ref().map(|view| &view.state),
                ) {
                    shared.channels.close(&id).await;
                    return Err(CoreError::bad_result(method, &reason));
                }
            }
            // file the session (a new one by hint, source, folder; a resumed one only if never seen).
            crate::projects_filing::loaded(shared, params, &state, kind == LoadKind::New).await;
            let applied = shared
                .store
                .load(state, kind)
                .map_err(|e| CoreError::bad_result(method, &e.to_string()))?;
            if kind != LoadKind::Read {
                shared.supervisor.track_session(&id);
                shared.access.observe_request(method, &id, params);
                if crate::working_note::mcp_disabled(&shared.instructions.own_home) {
                    shared.emit(crate::CoreEvent::Alert {
                        session_id: Some(id.clone()),
                        kind: crate::AlertKind::ToolsUnavailable,
                        message: crate::working_note::MCP_DISABLED.to_owned(),
                    });
                }
            }
            // Bounded channel: waits if the pump is behind; fails only when the pump is gone,
            // which happens only during shutdown.
            let _ = shared.loads.send(applied).await;
            // `/clear` and a rewind into a new session answer a new id (the channel already moved):
            // the session's resources follow it (`handoff`). A fork keeps its source's.
            if matches!(
                method,
                "session/rewind" | "session/compact" | "session/history/clear"
            ) && let Some(old) = source.filter(|s| *s != id)
            {
                crate::handoff::handed_off(shared, old, &id).await;
            }
            Ok(Some(id))
        }
    }
}

/// The store's snapshot of a loaded session.
///
/// # Errors
/// [`CoreError`] `notFound` when the session is not loaded.
pub(crate) fn snapshot(shared: &Shared, id: &str) -> Result<SessionSnapshot, CoreError> {
    shared
        .store
        .view(id)
        .map(|view| {
            let mut snapshot = SessionSnapshot::from(view);
            super::agent_tools::shell_commands::project(shared, &mut snapshot.state);
            crate::edit_preview::project(shared, &mut snapshot.state);
            shared.protected_changes.overlay(id, &mut snapshot.state);
            shared.chrome_control.overlay(id, &mut snapshot.state);
            crate::ask::tool_calls::overlay(shared, id, &mut snapshot.state);
            snapshot.access_mode =
                crate::restore::display_mode(shared, id, &snapshot.state["session"]);
            snapshot
        })
        .ok_or_else(|| CoreError::not_loaded(id))
}

/// The server result without its (large) `state`, for `CommandResult::Session::detail`.
pub(crate) fn detail(result: &Value) -> Value {
    let mut detail = result.clone();
    if let Some(object) = detail.as_object_mut() {
        object.remove("state");
    }
    detail
}

/// Native unified snapshots omit accepted extra roots and transient client observations. Normalize before
/// bookkeeping and before returning the reply, so the window and Debug API read the stream's meaning.
pub(crate) fn project_result(shared: &Shared, method: &str, params: &Value, result: &mut Value) {
    crate::provider_errors::value(result);
    super::agent_tools::shell_commands::project(shared, result);
    crate::edit_preview::project(shared, result);
    let Some(state) = result.get_mut("state").filter(|s| s.is_object()) else {
        return;
    };
    if method == "session/read" {
        *state = shared.store.project_read(std::mem::take(state));
    } else if matches!(effect(method), Effect::Load(_)) {
        let source = params["sessionId"]
            .as_str()
            .or_else(|| params["sourceSessionId"].as_str());
        let roots = params
            .pointer("/agentConfig/workspaceRoots")
            .cloned()
            .or_else(|| {
                source
                    .filter(|_| method != "session/resume")
                    .and_then(|id| shared.store.view(id))
                    .map(|v| v.state["session"]["workspaceRoots"].clone())
            });
        if state["session"]["workspaceRoots"]
            .as_array()
            .is_some_and(Vec::is_empty)
            && let Some(roots) = roots.filter(Value::is_array)
        {
            state["session"]["workspaceRoots"] = roots;
        }
    }
}
