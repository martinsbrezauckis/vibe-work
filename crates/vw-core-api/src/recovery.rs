//! Channel lifetime: attach on a person's command, read detached chats on view or server restart,
//! and close quiet unviewed channels. Depends on core, restore, sessions and channels; called by dispatch/pump.
//! A live channel belongs to this server generation and survives window/selection changes. A previous
//! generation's channel stays detached unless it has scheduled work. A schedule authorizes new turns,
//! with any unfinished old turn interrupted first; passive reads never resume it.
//! Explicit attaches restore the chosen profile before admitting new work. Idle closure preserves callbacks,
//! queued work, background processes, loops and never-run chats (Vibe has not stored those yet).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::command::{CommandResult, Origin};
use crate::core::{Core, Shared};
use crate::error::CoreError;
use crate::event::{AlertKind, CoreEvent};
use crate::sessions;

impl Core {
    /// Gives `session_id` a channel (a new one on the current server) by resuming it through the gate,
    /// loads the result, and restores its profile. A forced reopen for explicit retry;
    /// implicit sends use `attach_session`. Serialized through verification, without
    /// holding anything across sessions.
    pub(crate) async fn open_session(
        &self,
        origin: Origin,
        session_id: &str,
        bound: Duration,
    ) -> Result<(), CoreError> {
        let _held = self.shared.plugin_refresh.lock(session_id).await;
        self.reattach_session(origin, session_id, true, bound).await
    }

    /// Implicit opens coalesce: concurrent sends use the already verified channel.
    pub(crate) async fn attach_session(
        &self,
        origin: Origin,
        session_id: &str,
    ) -> Result<(), CoreError> {
        self.reattach_session(origin, session_id, false, self.shared.config.reopen_timeout)
            .await
    }

    async fn reattach_session(
        &self,
        origin: Origin,
        session_id: &str,
        force: bool,
        bound: Duration,
    ) -> Result<(), CoreError> {
        debug!(session_id, "opening a channel for the session");
        let canonical = self.shared.channels.canonical(session_id);
        let session_id = canonical.as_str();
        crate::interrupted_send::prepare(self, session_id).await?;
        let restore = self.begin_restore(session_id, None).await;
        if restore.attached && !force {
            return Ok(());
        }
        if !restore.attached {
            self.shared
                .holders
                .wait_for_handoff(&self.shared, session_id)
                .await?;
        }
        if !restore.attached
            && let Ok(Some(pid)) = self.session_holder(session_id).await
        {
            return Err(crate::session_holder::held_refusal(pid));
        }
        let _lease = self.claim_session(session_id).await?;
        let method = "session/resume";
        let stopping = self.shared.interrupted_send.pending(session_id).is_some();
        let (params, token) = restore.resume_request(stopping)?;
        crate::plan_profile::before_open(&self.shared, method, &params);
        let entry_project = crate::entry_projects::before_open(&self.shared, method, &params).await;
        let mut model_hold = crate::resume_models::begin(&self.shared, session_id, bound)?;
        let mut reply = match self
            .shared
            .gate
            .call(method, params.clone(), token.as_ref())
            .await
        {
            Ok(reply) => reply,
            Err(error) => {
                let error = crate::config_validation::from_gate(&self.shared, method, &error);
                if error.server_code.as_deref() == Some("sessionHeld") {
                    self.session_holder_after_refusal(session_id).await?;
                }
                return Err(error);
            }
        };
        crate::interrupted_send::stop_recovered(
            &self.shared,
            session_id,
            &mut reply.result,
            &mut model_hold,
        )
        .await?;
        if let Some(opening) = entry_project {
            opening.finish(&self.shared, &reply.result);
        }
        sessions::observe(&self.shared, method, &params, &reply.result).await?;
        self.restore_profile(origin, restore).await?;
        Box::pin(self.refresh_removed_plugins(session_id)).await
    }

    /// Selection keeps a live channel alive. A detached chat is only read: resuming it would recover
    /// unfinished work from an earlier process without a new turn from the person.
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
        if !self.shared.channels.has_channel(&id) {
            // Publish foreign ownership for the existing read-only UI without taking a lease.
            let _ = self.session_holder(&id).await;
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

/// Reads sessions whose channels died. Only saved schedules reopen: they authorize fresh turns.
/// The per-chat lock serializes recovery with explicit actions; an already attached live turn is left alone.
pub(crate) fn restore_detached(shared: &Arc<Shared>, inflight: Vec<(String, Vec<String>)>) {
    for (id, turns) in inflight {
        let core = Core {
            shared: shared.clone(),
        };
        tokio::spawn(async move {
            let _held = core.shared.plugin_refresh.lock(&id).await;
            if core.shared.channels.has_channel(&id) {
                return;
            }
            let applied = core.shared.store.mark_interrupted(&id, &turns);
            let _ = core.shared.loads.send(applied).await;
            let params = json!({"sessionId": id});
            let read = async {
                let mut reply = core
                    .shared
                    .gate
                    .call("session/read", params.clone(), None)
                    .await
                    .map_err(|error| CoreError::from_gate("session/read", &error))?;
                sessions::project_result(&core.shared, "session/read", &params, &mut reply.result);
                sessions::observe(&core.shared, "session/read", &params, &reply.result).await?;
                let scheduled = reply.result["state"]["session"]["archivedAt"].is_null()
                    && reply.result["state"]["session"]["loops"]
                        .as_array()
                        .is_some_and(|loops| !loops.is_empty());
                Ok::<_, CoreError>(scheduled)
            };
            let bound = core.shared.config.reopen_timeout;
            let result = tokio::time::timeout(bound, read).await.unwrap_or_else(|_| {
                Err(CoreError::new(
                    crate::ErrorCode::Timeout,
                    format!("no answer within {}s", bound.as_secs()),
                ))
            });
            // Let bounded_open own its deadline and channel cleanup; the read deadline must not cancel it.
            let result = match result {
                Ok(true) => {
                    core.bounded_open(&id, core.attach_session(Origin::Ui, &id))
                        .await
                }
                Ok(false) => Ok(()),
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => info!(
                    session_id = id,
                    "session restored detached after a server restart"
                ),
                Err(error) if error.server_code.as_deref() == Some("not_found") => {
                    // Vibe does not persist a never-run chat. Do not leave its dead row behind.
                    let applied = core.shared.store.remove(&id);
                    let _ = core.shared.loads.send(applied).await;
                    core.shared.leases.release(&id);
                }
                Err(error) => {
                    warn!(
                        session_id = id,
                        code = error.code.as_str(),
                        "reading a session after restart failed"
                    );
                    core.shared.emit(CoreEvent::Alert {
                        session_id: Some(id),
                        kind: AlertKind::ReopenFailed,
                        message: error.message,
                    });
                }
            }
        });
    }
}

/// The turns of each session that are `in_progress` in the store right now (before a detached restore).
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
