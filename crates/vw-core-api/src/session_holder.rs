//! A foreign app or CLI lease keeps a viewed chat read-only until its owner releases it.
//! Uses app locks and Vibe's lease-subscription snapshot, refreshed by the core poll, before reopening, and at once
//! when Vibe reports a lease change (`session/lease/updated`, via the pump): Vibe watches each holder's exit, so a
//! crashed window's chat unlocks when the OS releases its lease, not one poll later.
//! Each subscribe replaces the previous subscription and returns a fresh snapshot without taking a session.
//! Errors preserve the last observed state, and our own server is excluded. A session's reads and their publication
//! are serialized, so an older read never overwrites a newer one.
//! Called by core, dispatch, pump and `session_tools`; every server read is bounded.

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
/// Longest a read before reopening waits for a holder that has exited to lose its lease. A killed process keeps its
/// locks until its last thread has ended (its main thread already shows as a zombie meanwhile); Vibe reports the
/// release at once where it can watch the exit (Linux, macOS) and within its two-second poll elsewhere.
const RELEASE_WAIT: Duration = Duration::from_secs(3);

/// Checked sessions and their current foreign holders, plus the poll task owned by the core.
pub(crate) struct Holders {
    held: Mutex<BTreeMap<String, Option<(u32, String)>>>,
    poll: Mutex<Option<JoinHandle<()>>>,
    /// Counts Vibe's `session/lease/updated` reports; a read waiting for a release watches it.
    updates: tokio::sync::watch::Sender<u64>,
    /// Per session, held across one read and its publication.
    reading: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Default for Holders {
    fn default() -> Self {
        Self {
            held: Mutex::default(),
            poll: Mutex::default(),
            updates: tokio::sync::watch::Sender::new(0),
            reading: Mutex::default(),
        }
    }
}

impl Holders {
    /// Serializes `session_id`'s reads (another session's read, or its wait for a release, never delays it).
    async fn reading(&self, session_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .reading
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(session_id.to_owned())
            .or_default()
            .clone();
        lock.lock_owned().await
    }

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

    /// True while `session_id` is published as held by another process.
    fn shown_held(&self, session_id: &str) -> bool {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .is_some_and(Option::is_some)
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

/// Reads `session_id`'s holder; `Ok(None)` when free or held by our own server. With `wait`, a holder whose process
/// has already exited is read again after Vibe's next lease report (at most [`RELEASE_WAIT`]): its lease is still
/// held only while the OS finishes that exit. A running holder, an unknown pid (0) and a lease still held at the
/// deadline are returned as they are, so only the lock itself ever frees a chat.
async fn read_holder(
    shared: &Shared,
    session_id: &str,
    wait: bool,
) -> Result<Option<(u32, &'static str)>, CoreError> {
    let deadline = tokio::time::Instant::now() + RELEASE_WAIT;
    loop {
        // Before the read: a report that follows its snapshot is never missed.
        let mut updates = shared.holders.updates.subscribe();
        let holder = read_once(shared, session_id).await?;
        let Some((pid, _)) = holder else {
            return Ok(None);
        };
        if !wait || pid == 0 || tokio::time::Instant::now() >= deadline || !exited(pid).await {
            return Ok(holder);
        }
        let _ = tokio::time::timeout_at(deadline, updates.changed()).await;
    }
}

/// True when no running process has `pid` (gone, or a zombie whose remaining threads are still ending).
async fn exited(pid: u32) -> bool {
    tokio::task::spawn_blocking(move || vw_procmon::identity(pid).is_none())
        .await
        .unwrap_or(false)
}

/// One read of both leases: this app's other windows first, then Vibe's snapshot. The snapshot is read even when
/// another window holds the chat: that subscription is what makes Vibe report the release (the window's own server
/// holds Vibe's lease as long as the window holds the chat).
async fn read_once(
    shared: &Shared,
    session_id: &str,
) -> Result<Option<(u32, &'static str)>, CoreError> {
    let window = shared.leases.foreign(session_id)?;
    let method = "session/lease/subscribe";
    let reply = tokio::time::timeout(READ_WAIT, shared.gate.call(method, json!({}), None))
        .await
        .map_err(|_| CoreError::new(ErrorCode::Timeout, "the session's holder could not be read"))
        .and_then(|reply| reply.map_err(|error| CoreError::from_gate(method, &error)));
    if let Some(pid) = window {
        return Ok(Some((pid, "window")));
    }
    let Some(pid) = snapshot_holder(&reply?.result, session_id) else {
        return Ok(None);
    };
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

/// A held lease without a readable diagnostic still locks the chat (pid 0 means unknown).
fn snapshot_holder(snapshot: &Value, session_id: &str) -> Option<u32> {
    let lease = snapshot
        .get("held")?
        .as_array()?
        .iter()
        .find(|lease| lease["sessionId"].as_str() == Some(session_id))?;
    let holder = lease
        .get("leaseHolder")
        .filter(|holder| !holder.is_null())?;
    Some(
        holder
            .get("processId")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .unwrap_or(0),
    )
}

/// Checks one session and publishes a change. Errors leave the last known state.
async fn check(shared: &Shared, session_id: &str) {
    let _reading = shared.holders.reading(session_id).await;
    if shared.channels.has_channel(session_id) {
        // We hold it: never another process's.
        publish(shared, session_id, None);
    } else if let Ok(holder) = read_holder(shared, session_id, false).await {
        publish(shared, session_id, holder);
    }
}

/// Vibe reported a lease change (`session/lease/updated` on the lobby): wake any read waiting for a release, and
/// re-check the session now when it is shown held here. A new foreign holder of a free chat is still found by the
/// poll and before any reopen.
pub(crate) fn lease_updated(shared: &Arc<Shared>, session_id: Option<&str>) {
    shared.holders.updates.send_modify(|count| *count += 1);
    let Some(id) = session_id.filter(|id| shared.holders.shown_held(id)) else {
        return;
    };
    let shared = shared.clone();
    let id = id.to_owned();
    tokio::spawn(async move { check(&shared, &id).await });
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
        let _reading = self.shared.holders.reading(session_id).await;
        if self.shared.channels.has_channel(session_id) {
            return Ok(None);
        }
        let holder = read_holder(&self.shared, session_id, true).await?;
        publish(&self.shared, session_id, holder);
        Ok(holder.map(|(pid, _)| pid))
    }
}

#[cfg(test)]
mod tests {
    use super::snapshot_holder;
    use serde_json::json;

    #[test]
    fn lease_snapshots_distinguish_free_known_and_unknown_holders() {
        let snapshot = json!({"held":[
            {"sessionId":"known","leaseHolder":{"processId":42}},
            {"sessionId":"unknown","leaseHolder":{"processId":null}},
            {"sessionId":"free","leaseHolder":null}
        ]});
        assert_eq!(snapshot_holder(&snapshot, "known"), Some(42));
        assert_eq!(snapshot_holder(&snapshot, "unknown"), Some(0));
        assert_eq!(snapshot_holder(&snapshot, "free"), None);
        assert_eq!(snapshot_holder(&snapshot, "missing"), None);
    }
}
