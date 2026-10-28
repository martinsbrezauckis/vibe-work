//! Distinguishes subagent ownership from fork ancestry using Vibe's parent childSessions records.
//! Uses lobby reads (no leases); called by listing and delete admission.
use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::core::Shared;
use crate::{CoreError, ErrorCode};

const REFUSAL: &str = "Subagent chats are removed when you delete their parent chat.";

async fn read(shared: &Shared, id: &str) -> Result<Value, CoreError> {
    shared
        .gate
        .call(
            "session/read",
            json!({"sessionId":id,"history":{"limit":1}}),
            None,
        )
        .await
        .map(|reply| reply.result["state"].clone())
        .map_err(|error| CoreError::from_gate("session/read", &error))
}
fn contains(state: &Value, id: &str) -> bool {
    state["childSessions"]
        .as_array()
        .is_some_and(|rows| rows.iter().any(|row| row["id"] == id))
}
// Vibe 3.0.2's default catalogue omits SubagentSessionIdentity only (_host.py:3949–3953).
// Its parent-filtered catalogue includes forks too. Comparing those lists identifies a stored
// child even when the parent's stored projection omitted finished childSessions.
async fn catalog_contains(
    shared: &Shared,
    id: &str,
    key: &str,
    value: Option<&str>,
) -> Result<bool, CoreError> {
    let mut params = json!({"limit":500,"includeArchived":true});
    if let Some(value) = value {
        params[key] = Value::String(value.to_owned());
    }
    let mut cursors = BTreeSet::new();
    loop {
        let reply = shared
            .gate
            .call("session/list", params.clone(), None)
            .await
            .map_err(|error| CoreError::from_gate("session/list", &error))?;
        if reply.result["items"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["id"] == id))
        {
            return Ok(true);
        }
        let Some(next) = reply.result["nextCursor"].as_str() else {
            return Ok(false);
        };
        if !cursors.insert(next.to_owned()) {
            return Err(CoreError::new(
                ErrorCode::ServerError,
                "Vibe repeated a session-listing cursor",
            ));
        }
        params["cursor"] = Value::String(next.to_owned());
    }
}

/// Ownership from read/list fields, without attaching a runtime or guessing from an id's spelling.
pub(crate) fn owned_by_parent<'a>(
    shared: &'a Shared,
    session: &'a Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, CoreError>> + Send + 'a>> {
    Box::pin(owned_by_parent_inner(shared, session))
}

/// A parent id denotes ancestry; only the existing parent's child records denote ownership.
async fn owned_by_parent_inner(shared: &Shared, session: &Value) -> Result<bool, CoreError> {
    let (Some(parent), Some(id)) = (session["parentSessionId"].as_str(), session["id"].as_str())
    else {
        return Ok(false);
    };
    let state = match read(shared, parent).await {
        Ok(state) => state,
        Err(error) if error.server_code.as_deref() == Some("not_found") => return Ok(false),
        Err(error) => return Err(error),
    };
    // Finished children may be missing from a stored read; keep observed ownership while the parent exists.
    if contains(&state, id)
        || shared
            .store
            .view(parent)
            .is_some_and(|view| contains(&view.state, id))
    {
        return Ok(true);
    }
    // First prove the candidate is durable; absence from a root page alone would misclassify an
    // unsaved root. All pages include archived chats, and repeated cursors are refused.
    if !catalog_contains(shared, id, "parentSessionId", Some(parent)).await? {
        return Ok(false);
    }
    Ok(!catalog_contains(
        shared,
        id,
        "rootSessionId",
        session["rootSessionId"].as_str(),
    )
    .await?)
}
/// Keeps the shared dispatch future bounded while checking session-end admission.
pub(crate) fn check_delete<'a>(
    shared: &'a Shared,
    method: &'a str,
    params: &'a Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + 'a>> {
    Box::pin(check_delete_inner(shared, method, params))
}
/// Refuses only a still-owned child, for UI and raw Debug API deletes alike.
async fn check_delete_inner(
    shared: &Shared,
    method: &str,
    params: &Value,
) -> Result<(), CoreError> {
    crate::session_end::check_archive(shared, method, params)?;
    if method != "session/delete" {
        return Ok(());
    }
    let Some(id) = params["sessionId"].as_str() else {
        return Ok(());
    };
    let state = match read(shared, id).await {
        Ok(state) => state,
        // Damaged stores still need the existing confirmed deletion repair path. An
        // unreadable target provides no ownership evidence; the gate and no-follow cleanup still
        // enforce exact consent. This also preserves key-free startup cleanup of temporary chats.
        Err(error)
            if matches!(
                error.server_code.as_deref(),
                Some("not_found" | "internal_error")
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if owned_by_parent(shared, &state["session"]).await? {
        return Err(CoreError::new(ErrorCode::Denied, REFUSAL).with_method(method));
    }
    Ok(())
}
