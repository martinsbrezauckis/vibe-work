//! One controlled Vibe restart after a vault unlock, so the person's saved new-chat default is confirmed at once.
//!
//! A Vibe process that started with the vault locked has no usable model catalog: it reports the saved `active_model`
//! unpinned, and in the same process it only fetches the catalog again on its own retry schedule (measured with the
//! pinned 3.1.5: about 30 s after the failed fetch, longer after repeated failures); a `config/read` with a catalog
//! refresh or an attached `config/reload` does not fetch it. A new process fetches it at once. So after the person
//! unlocks, when the lobby still reports the saved default unpinned and no chat is busy, Core asks the supervisor for
//! one restart (its death path reopens the chats) and waits for the new server before the unlock answers. When a
//! chat is busy nothing restarts: the composer keeps its guard (no implicit first Send on an unconfirmed default)
//! and the reason is logged.
//!
//! Depends on: the key service status, `default_model` (saved alias), the gate's lobby `config/read`, `recovery` and
//! `stop_tree` (busy checks), the supervisor's `restart` and state. Called by: `key_replace` after `UnlockVault`.

use std::time::Duration;

use serde_json::{Value, json};
use tracing::{info, warn};
use vw_supervisor::SupervisorState;

use crate::core::Shared;

/// How long the unlock waits for the new server before answering anyway.
const RESTART_WAIT: Duration = Duration::from_secs(60);

/// Why a restart would cut something the person has going, if anything does.
fn busy(shared: &Shared) -> Option<(String, &'static str)> {
    shared.store.views().into_iter().find_map(|view| {
        let id = view.session_id;
        let reason = if crate::recovery::working(shared, &id) {
            "a turn, an approval card or a queued message is active"
        } else if crate::stop_tree::working(shared, &id) {
            "a stop is still in progress"
        } else if crate::recovery::never_ran(shared, &id) {
            "a new chat has not sent its first message yet"
        } else if view
            .state
            .pointer("/backgroundProcesses")
            .and_then(Value::as_array)
            .is_some_and(|all| all.iter().any(|p| p["status"] == "running"))
        {
            "a background process is running"
        } else {
            return None;
        };
        Some((id, reason))
    })
}

fn generation(shared: &Shared) -> Option<u64> {
    match &*shared.supervisor.state().borrow() {
        SupervisorState::Ready { generation, .. } => Some(*generation),
        _ => None,
    }
}

/// The check (module header), once per successful unlock. Never fails the unlock.
pub(crate) async fn after_unlock(shared: &Shared) {
    let usable = match shared.config.keys.as_deref() {
        Some(keys) => keys
            .status()
            .await
            .is_ok_and(|s| matches!(s.key, vw_keys::KeyPresence::Ready { .. })),
        None => false,
    };
    let Some(saved) = crate::default_model::saved_alias(shared).filter(|_| usable) else {
        return;
    };
    let lobby = match shared.gate.call("config/read", json!({}), None).await {
        Ok(reply) => reply.result,
        Err(error) => {
            warn!(%error, "saved default check after unlock: the lobby config could not be read");
            return;
        }
    };
    if lobby["config"]["activeModelPinned"] == true {
        return;
    }
    if let Some((session_id, reason)) = busy(shared) {
        info!(
            saved,
            session_id,
            reason,
            "saved default model still unconfirmed after unlock; Vibe is not restarted while a chat is busy; \
             new chats ask for a model choice until Vibe confirms it or Vibe Work restarts"
        );
        return;
    }
    let before = generation(shared);
    if !shared.supervisor.restart() {
        info!(
            saved,
            "saved default model unconfirmed after unlock; Vibe is not ready, so it is not restarted"
        );
        return;
    }
    info!(
        saved,
        "saved default model unconfirmed after unlock; restarting Vibe once so it reads the model catalog"
    );
    let mut state = shared.supervisor.state();
    let ready = tokio::time::timeout(
        RESTART_WAIT,
        state.wait_for(|s| {
            matches!(s, SupervisorState::Ready { generation, .. } if Some(*generation) != before)
                || matches!(
                    s,
                    SupervisorState::Failed { .. } | SupervisorState::Stopped { .. }
                )
        }),
    )
    .await;
    if !matches!(ready, Ok(Ok(_))) {
        warn!("Vibe did not come back within the unlock's wait after the requested restart");
    }
}
