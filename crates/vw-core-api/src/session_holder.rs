//! A session held by another process: show a session open elsewhere as read-only.
//!
//! How it is found: `session/read` answers `leaseHolder: {processId}` (2.26.1) from the lobby, without taking the
//! session (the holder's pid while the Vibe CLI or another app-server has it open, `null` right
//! after it exits, and a `session/resume` meanwhile answers `conflict` / `session_busy`). Vibe's push
//! (`session/lease/subscribe`) is not used: on Linux it fails with an `internal_error` (missing `os.pidfd_open`) under
//! Python builds without `os.pidfd_open`, and on Windows it has no exit watch
//! (a 10 s recheck). Instead the core asks for the sessions a window shows that no channel of ours holds, every
//! [`POLL`]; a change becomes the `sessionHolder` event, so the window locks and unlocks live.
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

/// Foreign holders known now (session id -> pid), and the poll task (owned by the core, aborted at shutdown).
#[derive(Default)]
pub(crate) struct Holders {
    held: Mutex<BTreeMap<String, u32>>,
    poll: Mutex<Option<JoinHandle<()>>>,
}

impl Holders {
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
    fn set(&self, session_id: &str, holder: Option<u32>) -> bool {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let before = held.get(session_id).copied();
        match holder {
            Some(pid) => {
                held.insert(session_id.to_owned(), pid);
            }
            None => {
                held.remove(session_id);
            }
        }
        before != holder
    }
}

/// The refusal for a call that would reopen a session another process holds.
pub(crate) fn held_refusal(pid: u32) -> CoreError {
    let mut error = CoreError::new(
        ErrorCode::Denied,
        format!(
            "This session is open in another Vibe process (pid {pid}), for example the Vibe CLI. It is read-only here \
             until that process closes it; it unlocks by itself then."
        ),
    );
    error.server_code = Some("sessionHeld".to_owned());
    error
}

/// Reads `session_id`'s lease holder from the lobby; `Ok(None)` when free or held by our own server.
async fn read_holder(shared: &Shared, session_id: &str) -> Result<Option<u32>, CoreError> {
    let page = json!({"limit": 1});
    let params = json!({"sessionId": session_id, "history": page, "turns": page});
    let reply = tokio::time::timeout(READ_WAIT, shared.gate.call("session/read", params, None))
        .await
        .map_err(|_| CoreError::new(ErrorCode::Timeout, "the session's holder could not be read"))?
        .map_err(|e| CoreError::from_gate("session/read", &e))?;
    let pid = reply
        .result
        .pointer("/leaseHolder/processId")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok());
    let ours = shared.supervisor.server_pid();
    Ok(pid.filter(|p| Some(*p) != ours))
}

/// Checks one session and publishes a change. Errors leave the last known state.
async fn check(shared: &Shared, session_id: &str) {
    if let Ok(holder) = read_holder(shared, session_id).await
        && shared.holders.set(session_id, holder)
    {
        shared.emit(CoreEvent::SessionHolder {
            session_id: session_id.to_owned(),
            holder_pid: holder,
        });
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
        let mut interval = tokio::time::interval(POLL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let Some(shared) = weak.upgrade() else { return };
            for id in shared.channels.viewed_ids() {
                if shared.channels.has_channel(&id) {
                    // We hold it: never another process's.
                    if shared.holders.set(&id, None) {
                        shared.emit(CoreEvent::SessionHolder {
                            session_id: id.clone(),
                            holder_pid: None,
                        });
                    }
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
        if self.shared.holders.set(session_id, holder) {
            self.shared.emit(CoreEvent::SessionHolder {
                session_id: session_id.to_owned(),
                holder_pid: holder,
            });
        }
        Ok(holder)
    }
}
