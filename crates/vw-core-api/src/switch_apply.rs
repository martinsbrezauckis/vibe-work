//! The Developer permissions switch reaches each open chat the way the access mode does (`restore.rs`): the switch
//! publishes the changed config and marks open chats stale, then idle chats reload in the background. Running
//! chats apply at turn end; app-forwarded work also checks before sending. Vibe-internal scheduled turns do not
//! take the app send lock: background apply is best effort, not an atomic scheduler fence. A detached chat reads
//! the changed config when it reopens.
//!
//! Depends on `channels` (per-channel switch generation), `recovery` (working / quiet), `stop_tree` (subagents)
//! and `plugin_refresh` (which methods build a runtime, the chat's send lock). Called by `dispatch_forward` (under
//! the chat's send lock, before work is sent), `developer_permissions` (after publication) and `pump` (turn ended).
//! Invariants:
//! - Nothing here or in the switch sends `mcp_catalog/refresh`: in pinned Vibe 3.1.4 that is a forced discovery
//!   (`app_server/mcp_catalog.py`, `force_remote_discovery=True`) that runs disabled command rows too.
//! - Apply = session `config/reload` while the chat is idle: Vibe rereads the files and marks MCP due, and the
//!   chat's next turn converges it at its own boundary without forcing discovery (servers the config disables are
//!   suspended). While ON, only enabled servers queued by the switch or shared config changes are applied through Vibe's
//!   non-forced per-server toggle (`switch_resume.rs`); a person-disabled row is never enabled or discovered.
//!   If applying fails, a quiet chat that ran is closed and the same send reopens
//!   it from the current config in its verified access mode. Otherwise the work is refused with
//!   `developerSwitchNotApplied` and an alert; the next command tries again.
//! - A chat with work in progress is never applied or cut; new work for it while it is stale is refused.
//! - The reload runs under the switch's mutation lock, so it reads one settled config and the generation it
//!   records is the one that config belongs to.

use std::sync::Arc;

use serde_json::{Value, json};
use tracing::warn;

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::event::{AlertKind, CoreEvent};

#[path = "switch_resume.rs"]
mod resume;

/// Agent work in progress that applying the change would cut: a turn, a waiting callback, a queued turn or a
/// restore (`recovery::working`), or a subagent (`stop_tree::working`).
fn busy(shared: &Shared, id: &str) -> bool {
    crate::recovery::working(shared, id) || crate::stop_tree::working(shared, id)
}

/// Reload an idle chat and, while ON, enable only resumed servers under the switch lock.
/// True only after every required request succeeded within one total apply timeout.
async fn reload(shared: &Arc<Shared>, id: &str, deadline: Option<tokio::time::Instant>) -> bool {
    let _mutation = shared.developer_permissions.mutation_lock().await;
    // A timeout polls its inner future first: a just-released mutation lock must not admit a late RPC.
    if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
        warn!(session_id = %id, "developer permissions: idle apply exhausted the shared switch budget");
        return false;
    }
    // The mutation lock may have been held by another chat while this one started work.
    if busy(shared, id) {
        return false;
    }
    let generation = shared.channels.switch_generation();
    let reload = async {
        shared
            .channels
            .request(
                "config/reload",
                json!({"sessionId": id, "reloadRuntime": true}),
            )
            .await
            .map_err(|error| CoreError::from_rpc("config/reload", &error))?;
        if shared.developer_permissions.enabled() && !resume::apply(shared, id).await {
            return Err(CoreError::new(
                ErrorCode::ServerUnavailable,
                vw_vocabulary::texts::DEVELOPER_SWITCH_NOT_APPLIED,
            ));
        }
        Ok(())
    };
    match tokio::time::timeout(shared.config.reopen_timeout, reload).await {
        Ok(Ok(())) => {
            shared.channels.switch_applied(id, generation);
            true
        }
        Ok(Err(error)) => {
            warn!(session_id = %id, %error, "developer permissions: a chat did not reload its config");
            false
        }
        Err(_) => {
            warn!(session_id = %id, "developer permissions: a chat's config reload timed out");
            false
        }
    }
}

fn refusal(shared: &Shared, id: &str, method: &str) -> CoreError {
    let message = vw_vocabulary::texts::DEVELOPER_SWITCH_NOT_APPLIED;
    shared.emit(CoreEvent::Alert {
        session_id: Some(id.to_owned()),
        kind: AlertKind::ToolsUnavailable,
        message: message.to_owned(),
    });
    let mut error = CoreError::new(ErrorCode::ServerUnavailable, message).with_method(method);
    error.server_code = Some("developerSwitchNotApplied".to_owned());
    error
}

/// Under the chat's send lock, before work that builds its runtime is sent: a chat whose channel predates the
/// last switch applies it first (module invariants), or the work is refused.
pub(crate) async fn before_work(
    shared: &Arc<Shared>,
    method: &str,
    params: &Value,
) -> Result<(), CoreError> {
    if !crate::plugin_refresh::builds_runtime(method) {
        return Ok(());
    }
    let Some(id) = params["sessionId"].as_str() else {
        return Ok(());
    };
    let id = shared.channels.canonical(id);
    if !shared.channels.switch_stale(&id) {
        return Ok(());
    }
    if busy(shared, &id) {
        return Err(refusal(shared, &id, method));
    }
    if reload(shared, &id, None).await {
        return Ok(());
    }
    if crate::recovery::quiet(shared, &id) {
        // The same send reopens the chat from the current config, in its verified access mode.
        Box::pin(shared.channels.close(&id)).await;
    }
    if shared.channels.switch_stale(&id) {
        return Err(refusal(shared, &id, method));
    }
    Ok(())
}

/// Refuse work if a switch landed during the dispatcher's awaited preparation after `before_work`.
/// Called under the same chat send lock, immediately before dispatch starts the transport call.
/// Once admitted here, a send already in flight may finish on the old setting after OFF publishes
/// (owner option B). Work reaching this check after OFF is refused; the chat applies OFF before its
/// next work.
pub(crate) fn at_send(shared: &Shared, method: &str, params: &Value) -> Result<(), CoreError> {
    if !crate::plugin_refresh::builds_runtime(method) {
        return Ok(());
    }
    let Some(id) = params["sessionId"].as_str() else {
        return Ok(());
    };
    let id = shared.channels.canonical(id);
    if shared.channels.switch_stale(&id) {
        return Err(refusal(shared, &id, method));
    }
    Ok(())
}

/// After publishing a switch, give every currently idle open chat one background apply opportunity.
/// All chats share one deadline, including send/mutation lock waits; one chat's send lock cannot hold up another chat.
/// Failed or timed-out reloads stay stale for the existing before-work recovery. Do not put close/reattach under
/// this cancellation deadline: resume can bind a channel before its access profile has been verified.
pub(crate) fn published(shared: &Arc<Shared>) {
    let ids: Vec<_> = shared
        .channels
        .open_channels()
        .into_iter()
        .map(|channel| channel.session_id)
        .filter(|id| shared.channels.switch_stale(id) && !busy(shared, id))
        .collect();
    if ids.is_empty() {
        return;
    }
    let deadline = tokio::time::Instant::now() + shared.config.reopen_timeout;
    let shared = shared.clone();
    tokio::spawn(async move {
        let mut workers = tokio::task::JoinSet::new();
        for id in &ids {
            let (shared, id) = (shared.clone(), id.clone());
            workers.spawn(async move {
                let apply = async {
                    let _held = shared.plugin_refresh.lock(&id).await;
                    if tokio::time::Instant::now() < deadline
                        && shared.channels.switch_stale(&id)
                        && !busy(&shared, &id)
                    {
                        reload(&shared, &id, Some(deadline)).await;
                    }
                };
                // Poll the deadline first: a late RPC answer must not advance ON to its next toggle.
                tokio::select! {
                    biased;
                    () = tokio::time::sleep_until(deadline) => {
                        warn!(session_id = %id, "developer permissions: idle apply exceeded the shared switch budget");
                    }
                    () = apply => {}
                }
            });
        }
        while let Some(result) = workers.join_next().await {
            if let Err(error) = result {
                warn!(%error, "developer permissions: idle apply worker failed");
            }
        }
    });
}

/// A stale chat's turn ended: apply the change now if the chat is idle (a loop or a later message then starts
/// from the current config). A failure leaves it stale for [`before_work`].
pub(crate) async fn after_turn(shared: Arc<Shared>, id: String) {
    let _held = shared.plugin_refresh.lock(&id).await;
    if shared.channels.switch_stale(&id) && !busy(&shared, &id) {
        reload(&shared, &id, None).await;
    }
}

#[cfg(test)]
#[path = "switch_apply_tests.rs"]
mod tests;
