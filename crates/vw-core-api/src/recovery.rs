//! Channel lifetime decided by the core: opening a channel for a session that needs one,
//! reopening the sessions that had a channel after a server restart, putting a resumed session back in
//! its profile, and closing the channel of a session that is quiet and not looked at.
//!
//! Why close at all: a session's channel holds the session's lease in Vibe; while it is open the
//! Vibe CLI cannot take the session ("Session is already open"), and each channel
//! costs memory in the server. Why only when quiet: closing a channel ends the session's work
//! (a turn blocked on an approval can be ended by the close).
//!
//! Depends on: `core` (`Core`, `Shared`: gate, store, channels, config), `restore`, `sessions` (store
//! effect of a resume). Called by: `dispatch` (a
//! channel on selection or before a call, the profile after a client's resume), `pump` (after a new server
//! generation; on its idle tick).
//! Invariants:
//! - A session is quiet when Vibe reports it idle, failed or archived, it has no open callback (in
//!   the store or the gate's book), no queued turn, no running background process, it is not being
//!   restored, no owned child is running or waiting on a card, and it has run at least one turn (a never-run
//!   session is not stored by Vibe: closing its channel would lose it).
//! - A quiet session keeps its channel while Vibe lists scheduled loops for it: Vibe runs loops only on an open
//!   channel (`close_idle`).
//! - Every resume the core sends names the resolved session profile and is
//!   followed by the profile restore (`restore.rs`, fail closed), so no turn ever runs in a mode the
//!   user did not choose.
//! - After a crash, turns that were `in_progress` and come back `completed` without output are marked
//!   `interrupted` (`cut_off`).
//! - Reopens go through the gate like any resume; a session Vibe no longer knows (`not_found`, e.g. a
//!   never-run session lost with the crash) is removed from the store, so no dead row lingers.
//! - Reopens and closes run in spawned tasks: the pump never waits for a server round trip.

use std::sync::Arc;

use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::command::{CommandResult, Origin};
use crate::core::{Core, Shared};
use crate::error::CoreError;
use crate::event::{AlertKind, CoreEvent};
use crate::sessions;

impl Core {
    /// Gives `session_id` a channel (a new one on the current server) by resuming it through the gate,
    /// loads the result, and restores its profile. A forced reopen for recovery and explicit retry;
    /// selection and implicit sends use `attach_session`. Serialized through verification, without
    /// holding anything across sessions.
    pub(crate) async fn open_session(
        &self,
        origin: Origin,
        session_id: &str,
    ) -> Result<(), CoreError> {
        let _held = self.shared.plugin_refresh.lock(session_id).await;
        self.reattach_session(origin, session_id, true).await
    }

    /// Implicit opens coalesce: a send racing selection uses the already verified channel.
    pub(crate) async fn attach_session(
        &self,
        origin: Origin,
        session_id: &str,
    ) -> Result<(), CoreError> {
        self.reattach_session(origin, session_id, false).await
    }

    async fn reattach_session(
        &self,
        origin: Origin,
        session_id: &str,
        force: bool,
    ) -> Result<(), CoreError> {
        debug!(session_id, "opening a channel for the session");
        let canonical = self.shared.channels.canonical(session_id);
        let session_id = canonical.as_str();
        let restore = self.begin_restore(session_id, None).await;
        if restore.attached && !force {
            return Ok(());
        }
        if !restore.attached
            && let Ok(Some(pid)) = self.session_holder(session_id).await
        {
            return Err(crate::session_holder::held_refusal(pid));
        }
        let _lease = self.claim_session(session_id).await?;
        let method = "session/resume";
        let params = json!({"sessionId": session_id, "agentConfig": {"agent": restore.profile()}});
        crate::plan_profile::before_open(&self.shared, method, &params);
        let entry_project = crate::entry_projects::before_open(&self.shared, method, &params).await;
        let reply = self
            .shared
            .gate
            .call(method, params.clone(), None)
            .await
            .map_err(|e| CoreError::from_gate(method, &e))?;
        if let Some(opening) = entry_project {
            opening.finish(&self.shared, &reply.result);
        }
        sessions::observe(&self.shared, method, &params, &reply.result).await?;
        self.restore_profile(origin, restore).await?;
        Box::pin(self.refresh_removed_plugins(session_id)).await
    }

    /// Selection keeps the channel alive and attaches immediately, using the same bounded,
    /// verified restore as a send. The listing already carries its mode for instant display.
    pub(crate) async fn view_session(
        &self,
        origin: Origin,
        session_id: Option<String>,
    ) -> Result<CommandResult, CoreError> {
        self.shared
            .channels
            .set_viewed(origin.as_str(), session_id.clone());
        let Some(id) = session_id else {
            return Ok(CommandResult::Reply {
                result: Value::Null,
                client_user_message_id: None,
                user_entry_ids: Vec::new(),
            });
        };
        if !self.shared.channels.has_channel(&id)
            && crate::stop_tree::owned(&self.shared, &id).await?
        {
            return self
                .session_call(origin, "session/read", json!({"sessionId":id}))
                .await;
        }
        let held = self.shared.plugin_refresh.lock(&id).await;
        if self.shared.plugin_refresh.needs_reopen(&id) {
            self.reopen_tools(&id).await?;
        }
        if self.shared.stuck.is_stuck(&id) {
            return Err(crate::stuck::refusal(&id, "session/resume"));
        }
        if let Err(error) = self
            .bounded_open(&id, self.attach_session(origin, &id))
            .await
        {
            if error.server_code.as_deref() != Some("sessionHeld") {
                return Err(error);
            }
            // The read-only read below goes through the dispatcher, which takes this chat's lock itself: holding
            // it here waited forever for a chat another Vibe holds (se1 H1).
            drop(held);
            if self.session_holder(&id).await?.is_none() {
                crate::session_holder::note_busy(&self.shared, &id);
            }
            return self
                .session_call(origin, "session/read", json!({"sessionId": id}))
                .await;
        }
        let actual = self.shared.channels.canonical(&id);
        self.shared
            .channels
            .set_viewed(origin.as_str(), Some(actual.clone()));
        Ok(CommandResult::Session {
            snapshot: sessions::snapshot(&self.shared, &actual)?,
            detail: Value::Null,
        })
    }
}

/// Resumes each session on a new channel of the new server (the old channels died with it), in the
/// profile it ran with. `inflight` lists, per session, the turns that were `in_progress` in the store
/// when the server died: the app knows they were cut off, so they are marked `interrupted` here
/// whatever the resumed state reports ([`cut_off`]); turns missing from the resumed state are marked
/// by the store itself.
pub(crate) fn reopen(shared: &Arc<Shared>, inflight: Vec<(String, Vec<String>)>) {
    for (id, turns) in inflight {
        let core = Core {
            shared: shared.clone(),
        };
        tokio::spawn(async move {
            let bound = if turns.is_empty() {
                core.shared.config.reopen_timeout
            } else {
                core.shared.config.inflight_reopen_timeout
            };
            let opened = tokio::time::timeout(bound, core.open_session(Origin::Ui, &id))
                .await
                .unwrap_or_else(|_| {
                    Err(CoreError::new(
                        crate::error::ErrorCode::Timeout,
                        format!("no answer to the reopen within {}s", bound.as_secs()),
                    ))
                });
            match opened {
                Ok(()) => {
                    info!(session_id = id, "session reopened after a server restart");
                    mark_cut_off(&core.shared, &id, &turns).await;
                }
                Err(error) if error.server_code.as_deref() == Some("not_found") => {
                    warn!(
                        session_id = id,
                        "session is gone after the restart; dropping it"
                    );
                    let applied = core.shared.store.remove(&id);
                    let _ = core.shared.loads.send(applied).await;
                    core.shared.leases.release(&id);
                }
                Err(error) if error.server_code.as_deref() == Some("sessionHeld") => {
                    let applied = core.shared.store.mark_interrupted(&id, &turns);
                    let _ = core.shared.loads.send(applied).await;
                    info!(
                        session_id = id,
                        "session remains read-only until its other holder closes"
                    );
                }
                Err(error) => {
                    warn!(
                        session_id = id,
                        code = error.code.as_str(),
                        "reopening a session after a restart failed"
                    );
                    // The turns that were running died with the server, whether or not the session
                    // can be reopened now.
                    let applied = core.shared.store.mark_interrupted(&id, &turns);
                    let _ = core.shared.loads.send(applied).await;
                    // No automatic retries (`stuck.rs`): the user decides.
                    core.note_stuck(&id);
                    if error.code == crate::ErrorCode::AccessModeUnverified {
                        return;
                    }
                    core.shared.emit(CoreEvent::Alert {
                        session_id: Some(id.clone()),
                        kind: AlertKind::ReopenFailed,
                        message: format!("{}: its unfinished turn was cut off. Try again, or continue in a new session.", crate::stuck::STUCK_TEXT),
                    });
                }
            }
        });
    }
}

/// The turns of `state` among `inflight` that a crash cut off and that did not finish: Vibe reports such a turn
/// completed without a reason, so the app's own record of in-flight turns says it was running. A turn whose last
/// entry is the model's answer finished (Vibe's resume told the model the cut-off step failed and the model went on):
/// it is not marked; its transcript still shows the failed step.
pub(crate) fn cut_off(state: &Value, inflight: &[String]) -> Vec<String> {
    let history = state["history"].as_array();
    let answered = |id: &str| {
        history
            .into_iter()
            .flatten()
            .rfind(|entry| entry["turnId"] == id)
            .is_some_and(|last| last["type"] == "message" && last["role"] == "assistant")
    };
    state["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let id = t["id"].as_str()?;
            let silent =
                t["status"] == "completed" && t["stopReason"].is_null() && t["error"].is_null();
            (inflight.iter().any(|x| x == id) && silent && !answered(id)).then(|| id.to_owned())
        })
        .collect()
}

/// Marks the cut-off turns of a reopened session interrupted and tells the user.
async fn mark_cut_off(shared: &Shared, id: &str, inflight: &[String]) {
    let Some(view) = shared.store.view(id) else {
        return;
    };
    let cut = cut_off(&view.state, inflight);
    if cut.is_empty() {
        return;
    }
    warn!(
        session_id = id,
        turns = cut.len(),
        "turns cut off by the restart marked interrupted"
    );
    let applied = shared.store.mark_interrupted(id, &cut);
    let _ = shared.loads.send(applied).await;
    shared.emit(CoreEvent::Alert {
        session_id: Some(id.to_owned()),
        kind: AlertKind::TurnsInterrupted,
        message: "Vibe stopped while this session was working; the unfinished turn was cut off."
            .to_owned(),
    });
}

/// The turns of each session that are `in_progress` in the store right now (before a reopen).
pub(crate) fn in_progress_turns(shared: &Shared, ids: Vec<String>) -> Vec<(String, Vec<String>)> {
    ids.into_iter()
        .map(|id| {
            let turns = shared.store.view(&id).map_or_else(Vec::new, |view| {
                view.state["turns"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|t| t["status"] == "in_progress")
                    .filter_map(|t| t["id"].as_str().map(str::to_owned))
                    .collect()
            });
            (id, turns)
        })
        .collect()
}

/// Closes the channels of quiet sessions that nobody views and that were unused for
/// `CoreConfig::channel_idle_close` (or were archived).
pub(crate) fn close_idle(shared: &Arc<Shared>) {
    let idle = shared.config.channel_idle_close;
    let ids = shared.channels.idle(idle, |id| quiet(shared, id));
    for id in ids {
        let shared = shared.clone();
        tokio::spawn(async move {
            let _held = shared.plugin_refresh.lock(&id).await;
            if !quiet(&shared, &id) {
                return;
            }
            // Vibe runs a session's loops only while its channel is open, so a session with loops stays open.
            if has_loops(&shared, &id).await {
                return;
            }
            if shared.channels.close(&id).await {
                info!(session_id = id, "closed the channel of a quiet session");
            }
        });
    }
}

/// Whether Vibe holds scheduled loops for `id` (`loops/list`; Vibe sends no events for loops). An unanswered
/// or malformed list counts as none: the session closes as it did before loops existed.
async fn has_loops(shared: &Arc<Shared>, id: &str) -> bool {
    let list = shared
        .channels
        .request("loops/list", serde_json::json!({ "sessionId": id }));
    tokio::time::timeout(std::time::Duration::from_secs(5), list)
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|answer| {
            answer
                .get("loops")
                .and_then(Value::as_array)
                .map(|all| !all.is_empty())
        })
        .unwrap_or(false)
}

/// After the Mistral key changed: Vibe Work gives every open session a fresh session runtime, so
/// each session starts over with the new key behind the proxy. Quiet, unviewed sessions close now (the
/// next command reopens them), busy ones when they become quiet. A viewed chat keeps its channel
/// until the next work request honors the pending close under its send lock (`plugin_refresh`).
pub(crate) async fn reopen_after_key_change(shared: &Arc<Shared>) {
    for open in shared.channels.open_channels() {
        let id = open.session_id;
        let _held = shared.plugin_refresh.lock(&id).await;
        if quiet(shared, &id) && !shared.channels.viewed_ids().contains(&id) {
            if shared.channels.close(&id).await {
                info!(
                    session_id = id,
                    "closed a session's channel after a key change"
                );
            }
        } else {
            shared.channels.close_when_quiet(&id);
        }
    }
}

/// See the module invariants.
pub(crate) fn quiet(shared: &Shared, id: &str) -> bool {
    let Some(view) = shared.store.view(id) else {
        return true; // nothing loaded: nothing of ours runs on it
    };
    let state = &view.state;
    let background_running = state
        .pointer("/backgroundProcesses")
        .and_then(Value::as_array)
        .is_some_and(|all| all.iter().any(|p| p["status"] == "running"));
    !working(shared, id)
        && !crate::stop_tree::working(shared, id)
        && !never_ran(shared, id)
        && !background_running
}

/// A loaded chat with no turn in the store: Vibe keeps it only in memory, so closing its channel discards it.
pub(crate) fn never_ran(shared: &Shared, id: &str) -> bool {
    shared.store.view(id).is_some_and(|view| {
        view.state
            .pointer("/turns")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    })
}

/// True while the session does something that closing its channel or ending its resources would cut: Vibe reports
/// it neither idle, failed nor archived (a turn runs), a callback waits or its answer is still being delivered
/// (store or the gate's book), a turn is queued, or the core is restoring its profile. A session that is not
/// loaded works on nothing of ours. This is also the condition an archived session's deferred teardown waits
/// for (`session_end.rs`).
pub(crate) fn working(shared: &Shared, id: &str) -> bool {
    if shared.channels.is_restoring(id) {
        return true;
    }
    let Some(view) = shared.store.view(id) else {
        return false;
    };
    let state = &view.state;
    let status = state
        .pointer("/session/status/type")
        .or_else(|| state.pointer("/session/status"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let waiting = |pointer: &str| {
        state
            .pointer(pointer)
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    };
    let gate_open = shared
        .gate
        .open_callbacks()
        .iter()
        .any(|c| c.session_id == id)
        || shared.gate.answering(id);
    !matches!(status, "idle" | "failed" | "archived")
        || waiting("/activeCallbacks")
        || waiting("/turnQueue/items")
        || gate_open
}

#[cfg(test)]
mod tests {
    use super::cut_off;
    use serde_json::json;

    #[test]
    fn inflight_turns_that_did_not_finish_are_cut_off() {
        let state = json!({
            "turns": [
                {"id": "cut", "status": "completed", "stopReason": null, "error": null},
                {"id": "answered", "status": "completed", "stopReason": null, "error": null},
                {"id": "stopped", "status": "completed", "stopReason": "max_turns", "error": null},
                {"id": "failed", "status": "failed", "stopReason": null, "error": {"message": "x"}},
                {"id": "older", "status": "completed", "stopReason": null, "error": null},
                {"id": "step", "status": "completed", "stopReason": null, "error": null},
                {"id": "redone", "status": "completed", "stopReason": null, "error": null}
            ],
            "history": [
                {"type": "message", "role": "user", "turnId": "cut"},
                {"type": "message", "role": "assistant", "turnId": "answered"},
                {"type": "message", "role": "user", "turnId": "step"},
                {"type": "message", "role": "assistant", "turnId": "step"},
                {"type": "effect", "turnId": "step"},
                {"type": "message", "role": "user", "turnId": "redone"},
                {"type": "effect", "turnId": "redone"},
                {"type": "effect", "turnId": "redone"},
                {"type": "message", "role": "assistant", "turnId": "redone"}
            ]
        });
        let inflight: Vec<String> = ["cut", "answered", "stopped", "failed", "step", "redone"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(
            cut_off(&state, &inflight),
            ["cut", "step"],
            "a turn the model answered after the resume finished; only the one that did not is cut off"
        );
        assert!(
            cut_off(&state, &[]).is_empty(),
            "only turns that were in flight"
        );
    }
}
