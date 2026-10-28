//! The end of a session, with one path for archive and delete.
//!
//! A deleted session is gone at once: its MCP credential dies, its terminals and own processes stop, it leaves the
//! store, its preview data is wiped, its page and file viewers close, it leaves its project, its review baseline
//! and its copy record are updated, and the agent processes it started move to the "left running" list
//! (`left_running.rs` asks the user about them). An archived session may still be finishing a turn ("closing now
//! would end a running turn"), so its channel closes when it is quiet, and its resources (terminals, own processes,
//! preview, page and file viewers, lease) end at that moment too, not mid-turn. Unarchiving before that cancels the
//! wait. Archive is refused before Vibe for a quiet chat without a turn: Vibe has not saved it, so
//! an archive could not be undone after restarting. Confirmed delete still ends such a chat.
//!
//! Depends on: `core::Shared`, `processes`, `terminals`, `previews`, `pages`, `files`, `projects_filing`,
//! `worktrees`, `recovery` (`working`), `names`. Called by: `sessions::observe` (delete, for typed commands and the
//! generic `call` alike, archive and unarchive), `pump` (its idle tick runs [`run_pending`]).
//! Invariants:
//! - Each resource is ended exactly once per end: a pending archive teardown is dropped when the session is
//!   deleted, unarchived, or torn down.
//! - The titles kept for the "left running" list are pruned to sessions that still have a process on it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use tracing::info;
use vw_gate::{GateError, Reply};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// Archive must be undoable: Vibe has no saved chat before its first turn. Unarchive remains allowed.
pub(crate) fn check_archive(
    shared: &Shared,
    method: &str,
    params: &Value,
) -> Result<(), CoreError> {
    if method == "session/archive"
        && params.get("archived") == Some(&Value::Bool(true))
        && never_ran(shared, params).is_some()
    {
        return Err(CoreError::new(
            ErrorCode::Denied,
            "This chat hasn't started yet, so there is nothing to archive. Delete it instead.",
        )
        .with_method(method));
    }
    Ok(())
}

/// Ends an archived session's resources when quiet; unarchive cancels a pending teardown.
pub(crate) async fn archive_decision(shared: &Shared, params: &Value) {
    let (Some(id), Some(archived)) = (
        params.get("sessionId").and_then(Value::as_str),
        params.get("archived").and_then(Value::as_bool),
    ) else {
        return;
    };
    if archived {
        archived_session(shared, id).await;
    } else {
        unarchived(shared, id);
    }
}

/// Vibe keeps a chat on disk only after its first turn, so a chat that never ran is unknown to it (`not_found`):
/// a confirmed delete of such a chat still completes and stops its terminal and processes.
/// Other missing sessions and methods stay errors.
pub(crate) fn missing_reply(
    shared: &Shared,
    method: &str,
    params: &Value,
    reply: Result<Reply, GateError>,
) -> Result<Reply, GateError> {
    match reply {
        Err(error)
            if CoreError::from_gate(method, &error).server_code.as_deref() == Some("not_found") =>
        {
            if method == "session/delete" && never_ran(shared, params).is_some() {
                return Ok(Reply {
                    result: json!({}),
                    client_user_message_id: None,
                    user_entry_ids: Vec::new(),
                });
            }
            Err(error)
        }
        other => other,
    }
}

/// The core's copy of a quiet chat that has no turn: one Vibe never stored.
fn never_ran(shared: &Shared, params: &Value) -> Option<Value> {
    let id = params.get("sessionId").and_then(Value::as_str)?;
    let view = shared.store.view(id)?;
    let quiet = !matches!(
        view.state
            .pointer("/session/status/type")
            .and_then(Value::as_str),
        Some("running" | "blocked")
    );
    (quiet
        && view
            .state
            .get("turns")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty))
    .then_some(view.state)
}

/// Session-end bookkeeping in the core.
#[derive(Default)]
pub(crate) struct Ends {
    /// Archived sessions whose teardown waits until they are quiet.
    pending: Mutex<BTreeSet<String>>,
    /// Titles of deleted sessions, for the "left running" list (pruned on read).
    titles: Mutex<BTreeMap<String, String>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Ends {
    /// The title a deleted session had, if one was known.
    pub(crate) fn title(&self, session_id: &str) -> Option<String> {
        locked(&self.titles).get(session_id).cloned()
    }

    /// Keeps only the titles of `live` sessions (those that still have a process on the list).
    pub(crate) fn keep_titles(&self, live: &BTreeSet<&str>) {
        locked(&self.titles).retain(|id, _| live.contains(id.as_str()));
    }
}

/// The subagents Vibe owns beneath a root, before deletion removes their durable index.
/// Vibe 3's read of a stored root can omit finished children that its earlier events reported;
/// retain that ownership evidence from our snapshot as well as the current server reply.
/// A fork also has `parentSessionId`, but remains an independent chat: only `childSessions` denotes ownership.
pub(crate) async fn deletion_children(
    shared: &Shared,
    method: &str,
    params: &Value,
) -> Vec<String> {
    let Some(root) = params["sessionId"]
        .as_str()
        .filter(|_| method == "session/delete")
    else {
        return Vec::new();
    };
    let mut seen = BTreeSet::from([root.to_owned()]);
    let mut pending = vec![root.to_owned()];
    let mut children = Vec::new();
    while let Some(id) = pending.pop() {
        let cached = shared
            .store
            .view(&id)
            .map_or(Value::Null, |view| view.state);
        let state = match shared
            .gate
            .call(
                "session/read",
                json!({"sessionId":id,"history":{"limit":1}}),
                None,
            )
            .await
        {
            Ok(reply) => reply.result["state"].clone(),
            Err(_) => Value::Null,
        };
        for child in [&cached, &state]
            .into_iter()
            .flat_map(|state| state["childSessions"].as_array().into_iter().flatten())
            .filter_map(|row| row["id"].as_str())
        {
            if seen.insert(child.to_owned()) {
                pending.push(child.to_owned());
                children.push(child.to_owned());
            }
        }
    }
    children.reverse();
    children
}

/// A session was deleted (Vibe confirmed `session/delete`): everything of it ends now.
pub(crate) async fn deleted(shared: &Shared, id: &str) {
    locked(&shared.ends.pending).remove(id);
    // The title is read before the store forgets the session (the "left running" list names it).
    if let Some(title) = shared.names.title_of(&shared.store, id) {
        locked(&shared.ends.titles).insert(id.to_owned(), title);
    }
    shared.access.forget(id);
    shared.queue_returns.forget(id);
    shared.model_choices.forget(id);
    shared.plugin_refresh.forget(id);
    shared.entry_projects.forget(id);
    // Its "Allow for this session" answers end with it.
    crate::ask::grants::session_ended(shared, id);
    // A deleted temporary session leaves the list (whoever deleted it).
    shared.temporary.remove_saved(id).await;
    shared.deletes.forget(id).await;
    shared.confirmer.deletion_finished(id);
    shared.search.forget(id);
    // The deleted session's MCP credential stops working at once.
    if let Some(tools) = &shared.agent_tools {
        tools.session_deleted(id);
    }
    // A deleted session's terminals stop with it, before the store forgets it.
    crate::terminals::close_session(shared, id).await;
    let applied = shared.store.remove(id);
    shared.saved_documents.forget(id);
    shared.supervisor.untrack_session(id);
    let _ = shared.loads.send(applied).await;
    // stop the session's own (`owned`) processes, return the lease; its agent processes
    // move to the "left running" list, which the window asks about next.
    let report = crate::processes::end_session(shared, id, true).await;
    if report.is_some_and(|r| r.agent_running.is_empty()) {
        locked(&shared.ends.titles).remove(id);
    }
    // its preview closes and its website data is wiped.
    crate::previews::session_ended(shared, id, true).await;
    // its page viewer goes (comments are the project's and stay).
    crate::pages::session_ended(shared, id).await;
    // it leaves its project (the store forgets it).
    crate::projects_filing::deleted(shared, id).await;
    crate::files::session_ended(shared, id).await;
    // A deleted session's review baseline goes with it.
    shared.changes.forget(id, true);
    locked(&shared.git.repos).remove(id);
    // its copy (if any) may now be an orphan, listed at the next start.
    crate::worktrees::session_deleted(shared, id);
}

/// A session was archived (Vibe confirmed it). Its channel closes when it is quiet; its resources end then too, or
/// now when nothing of it is running.
async fn archived_session(shared: &Shared, id: &str) {
    shared.channels.close_when_quiet(id);
    if ready(shared, id) {
        teardown_archived(shared, id).await;
    } else {
        info!(
            session_id = id,
            "archived while working: its resources end when it is quiet"
        );
        locked(&shared.ends.pending).insert(id.to_owned());
    }
}

/// A session was taken out of the archive: a teardown still waiting is dropped (the session is in use again).
pub(crate) fn unarchived(shared: &Shared, id: &str) {
    locked(&shared.ends.pending).remove(id);
}

/// The pump's idle tick: ends the resources of archived sessions that became quiet (or lost their channel).
pub(crate) fn run_pending(shared: &Arc<Shared>) {
    let due: Vec<String> = {
        let pending = locked(&shared.ends.pending);
        pending
            .iter()
            .filter(|id| ready(shared, id))
            .cloned()
            .collect()
    };
    for id in due {
        // Taken out first, so a later tick never starts a second teardown of the same session.
        if !locked(&shared.ends.pending).remove(&id) {
            continue;
        }
        let shared = shared.clone();
        tokio::spawn(async move {
            teardown_archived(&shared, &id).await;
        });
    }
}

/// True when an archived session's resources may end: it has no open channel (nothing of it runs through us), or
/// its channel is quiet in the sense of `recovery::working`.
fn ready(shared: &Shared, id: &str) -> bool {
    !shared.channels.is_restoring(id)
        && (!shared.channels.has_channel(id) || !crate::recovery::working(shared, id))
}

/// The resources of an archived session end (it may be restored later, so its preview's website data stays).
async fn teardown_archived(shared: &Shared, id: &str) {
    // An archived chat is closed: its "Allow for this session" answers end (a restored chat asks again).
    crate::ask::grants::session_ended(shared, id);
    // The session's own processes stop, the agent's are listed, the lease returns.
    crate::processes::end_session(shared, id, false).await;
    // An archived session's terminals stop with it.
    crate::terminals::close_session(shared, id).await;
    // its preview closes (website data kept: the session can be restored).
    crate::previews::session_ended(shared, id, false).await;
    // its page viewer goes.
    crate::pages::session_ended(shared, id).await;
    crate::files::session_ended(shared, id).await;
}
