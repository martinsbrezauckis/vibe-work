//! A session held by another process: show a session open elsewhere as read-only.
//!
//! How it is found: `session/read` answers `leaseHolder: {processId}` from the lobby, without taking the
//! session (the holder's pid while the Vibe CLI or another app-server has it open, `null` right
//! after it exits, and a `session/resume` meanwhile answers `conflict` / `session_busy`). Vibe Work polls instead
//! of subscribing to Vibe's push (`session/lease/subscribe`), so one code path serves every platform and Python
//! build: the core asks for the sessions a window shows that no channel of ours holds, every
//! [`POLL`] (`CoreConfig::holder_poll`); a change becomes the `sessionHolder` event, so the window locks and unlocks live.
//! The holder is "another process" only when its pid is not our own `vibe-app-server` (all our channels share it).
//!
//! Depends on: the gate (`session/read`), `channels` (viewed sessions, bound channels), the supervisor (our server's
//! pid). Called by: `core` (the poll task), `dispatch` (`forward`: a call that would reopen a held session is refused
//! with the holder named, instead of marking the session stuck), `session_tools` (`holder`).
//! Invariants: never takes a session (only lobby reads); every read is time-limited; the known holders are only
//! foreign ones.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;

/// How often viewed sessions are checked.
pub(crate) const POLL: Duration = Duration::from_secs(2);
/// Longest one holder read may take.
const READ_WAIT: Duration = Duration::from_secs(5);

/// Checked sessions and their current foreign holders, plus the poll task owned by the core.
#[derive(Default)]
pub(crate) struct Holders {
    held: Mutex<BTreeMap<String, Option<(u32, String)>>>,
    poll: Mutex<Option<JoinHandle<()>>>,
}

impl Holders {
    /// Current ownership for a fresh subscription or a subscriber that missed events.
    pub(crate) fn events(&self) -> Vec<CoreEvent> {
        let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        held.iter()
            .map(|(session_id, holder)| CoreEvent::SessionHolder {
                session_id: session_id.to_owned(),
                holder_pid: holder.as_ref().map(|(pid, _)| *pid),
                holder_kind: holder.as_ref().map(|(_, kind)| kind.clone()),
            })
            .collect()
    }
    /// Aborts the poll task (core shutdown).
    pub(crate) fn stop_poll(&self) {
        let poll = self
            .poll
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(poll) = poll {
            poll.abort();
        }
    }

    /// Records `holder` for `session_id`; true when it changed.
    fn set(&self, session_id: &str, holder: Option<u32>, kind: &str) -> bool {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let before = held.get(session_id).cloned().flatten();
        let after = holder.map(|pid| (pid, kind.to_owned()));
        held.insert(session_id.to_owned(), after.clone());
        before != after
    }
}

/// The refusal for a call that would reopen a session another process holds.
pub(crate) fn held_refusal(pid: u32) -> CoreError {
    let mut error = CoreError::new(
        ErrorCode::Denied,
        format!(
            "{} is open in another window (pid {pid}). Read-only here until it closes the session.",
            vw_vocabulary::text("thisSession").unwrap_or_default()
        ),
    );
    error.server_code = Some("sessionHeld".to_owned());
    error
}

/// Reads `session_id`'s lease holder from the lobby; `Ok(None)` when free or held by our own server.
async fn read_holder(
    shared: &Shared,
    session_id: &str,
) -> Result<Option<(u32, &'static str)>, CoreError> {
    if let Some(pid) = shared.leases.foreign(session_id)? {
        return Ok(Some((pid, "window")));
    }
    let page = json!({"limit": 1});
    let params = json!({"sessionId": session_id, "history": page, "turns": page});
    let reply = tokio::time::timeout(READ_WAIT, shared.gate.call("session/read", params, None))
        .await
        .map_err(|_| {
            CoreError::new(ErrorCode::Timeout, "the session's holder could not be read")
        })?;
    let reply = match reply {
        Ok(reply) => reply,
        // Deleted and never-stored sessions have no Vibe holder. Their command handler still
        // decides whether a local operation is valid or the session is missing.
        Err(error)
            if CoreError::from_gate("session/read", &error)
                .server_code
                .as_deref()
                == Some("not_found") =>
        {
            return Ok(None);
        }
        Err(error) => return Err(CoreError::from_gate("session/read", &error)),
    };
    let pid = reply
        .result
        .pointer("/leaseHolder/processId")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok());
    let Some(pid) = pid else { return Ok(None) };
    let ours = if let Some(leader) = shared.supervisor.server_pid() {
        let program = shared.supervisor.program();
        let launchers = shared.supervisor.launcher_programs();
        tokio::task::spawn_blocking(move || {
            crate::own_server::is_server(pid, leader, &program, &launchers)
        })
        .await
        .unwrap_or(false)
    } else {
        false
    };
    Ok((!ours).then_some((pid, "terminal")))
}

/// Checks one session and publishes a change. Errors leave the last known state.
async fn check(shared: &Shared, session_id: &str) {
    if let Ok(holder) = read_holder(shared, session_id).await {
        publish(shared, session_id, holder);
    }
}

fn publish(shared: &Shared, session_id: &str, holder: Option<(u32, &str)>) {
    let pid = holder.map(|(pid, _)| pid);
    let kind = holder.map_or("window", |(_, kind)| kind);
    if shared.holders.set(session_id, pid, kind) {
        shared.emit(CoreEvent::SessionHolder {
            session_id: session_id.to_owned(),
            holder_pid: pid,
            holder_kind: pid.map(|_| kind.to_owned()),
        });
    }
}

/// A resume can lose a race with the CLI after a free holder read. Keep the UI read-only while the
/// native diagnostic catches up; the next ordinary poll resolves its pid or releases this state.
pub(crate) fn note_busy(shared: &Shared, session_id: &str) {
    publish(shared, session_id, Some((0, "terminal")));
}

impl Core {
    pub(crate) async fn claim_session(
        &self,
        id: &str,
    ) -> Result<Option<Arc<crate::session_lease::Lease>>, CoreError> {
        match self.shared.leases.claim(id) {
            Ok(lease) => Ok(lease),
            Err(error) => {
                if error.server_code.as_deref() == Some("sessionHeld") {
                    let _ = self.session_holder(id).await;
                }
                Err(error)
            }
        }
    }
}

/// Starts the poll task (owned by the core, aborted at shutdown): the viewed sessions no channel of ours holds.
pub(crate) fn start_poll(shared: &Arc<Shared>) {
    let task = spawn_poll(shared);
    *shared
        .holders
        .poll
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(task);
}

fn spawn_poll(shared: &Arc<Shared>) -> JoinHandle<()> {
    let weak = Arc::downgrade(shared);
    tokio::spawn(async move {
        let every = weak
            .upgrade()
            .map_or(POLL, |shared| shared.config.holder_poll);
        let mut interval = tokio::time::interval(every);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let Some(shared) = weak.upgrade() else { return };
            for id in shared.channels.viewed_ids() {
                if shared.channels.has_channel(&id) {
                    // We hold it: never another process's.
                    publish(&shared, &id, None);
                    continue;
                }
                check(&shared, &id).await;
            }
        }
    })
}

impl Core {
    /// The foreign holder of `session_id`, read now (`sessionTools holder`, and before a call would reopen it).
    ///
    /// # Errors
    /// The read's error (timeout, gate refusal, Vibe error).
    pub(crate) async fn session_holder(&self, session_id: &str) -> Result<Option<u32>, CoreError> {
        if self.shared.channels.has_channel(session_id) {
            return Ok(None);
        }
        let holder = read_holder(&self.shared, session_id).await?;
        publish(&self.shared, session_id, holder);
        Ok(holder.map(|(pid, _)| pid))
    }
}
