//! A channel per active session: which multiplexed channel carries which session, and the
//! routing of every gated request to the right one.
//!
//! One `vibe-app-server` stdio connection has exactly one attached session. `vibe-app-server --multiplex`
//! serves many channels in one process, each its own server with its own attached session, events
//! and callbacks. So every session that is in use
//! gets its own channel; everything else (listing, reading and metadata of stored sessions, config,
//! account, catalogs) goes to the supervisor's lobby channel.
//!
//! Routing (`routing::route`):
//! - `session/start`, `session/continue`: a new channel, bound to the session id of the result
//!   (a second start on a channel would replace its session).
//! - `session/resume`: the session's channel if it has one (re-attach), else a new channel bound on
//!   success. Resuming on a channel that holds another session would replace it, so never done.
//! - `session/delete`: the session's channel is closed first (it holds the session lease), then the
//!   delete goes to the lobby.
//! - any other request naming a session (`sessionId`, or `sourceSessionId` for a fork): that
//!   session's channel when it has one, else the lobby. The lobby can read and change the metadata
//!   of stored sessions; a session open on a channel is answered by the lobby from disk only
//!   (different event numbers, no notification to the owner), so the owner's channel is used.
//!   `callback/result` for a session without a channel (a subagent's card names its child session
//!   but arrives on the parent's channel) goes to the channel the callback arrived on; Vibe answers a child's
//!   callback only on the channel its parent is attached to; the lobby refuses it.
//! - everything else: the lobby.
//! - A result whose `state.session.id` differs from the session it was sent for (a fork that
//!   attaches, a rewind or `/clear` that hands off to a new id) moves the channel to the new id; a
//!   detached fork (`attach: false`) does not (`routing::may_hand_off`).
//!
//! Depends on: vw-supervisor (`open_channel`, `resume_on`, lobby), vw-rpc, `routing`. Called by:
//! `transport` (the gate's transport), `pump` (subscriptions, restarts, events), `recovery` (idle
//! close, restore), `dispatch` (view, archive).
//! Invariants:
//! - A session is bound to at most one channel, and a channel to one session.
//! - Opening, resuming into and closing a session's channel are serialized per session (a per-session
//!   async lock); nothing is ever serialized across sessions.
//! - A channel is subscribed (and the subscription handed to the pump) before its first session
//!   request, so no event of the session is missed.
//! - Bindings carry the server generation; a restart drops them all (`reset`). A binding whose
//!   channel is dead counts as no channel (a command then opens one on the new server).
//! - A session being restored to its profile (`recovery.rs`) counts as having no channel.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, warn};
use vw_rpc::{Connection, DeathCause, RpcError, Subscription};
use vw_supervisor::{Supervisor, SupervisorError};

use crate::routing::{Route, may_hand_off, route};

/// Identifies one channel across restarts: (server generation, channel id).
pub(crate) type ChannelKey = (u64, u64);

/// A new channel's subscription, for the pump.
pub(crate) struct Feed {
    pub(crate) key: ChannelKey,
    pub(crate) sub: Subscription,
}

/// One session's channel.
struct Slot {
    conn: Connection,
    key: ChannelKey,
    last_used: Instant,
}

impl Slot {
    fn alive(&self) -> bool {
        self.conn.death().is_none()
    }
}

#[derive(Default)]
struct Inner {
    slots: HashMap<String, Slot>,
    /// The session each client's main view shows (`Command::ViewSession`), by origin name.
    viewed: BTreeMap<&'static str, String>,
    /// Sessions whose channel should close as soon as they are quiet (archived).
    close_when_quiet: BTreeSet<String>,
}

/// One open channel as the status report shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenChannel {
    pub(crate) session_id: String,
    pub(crate) channel_id: u64,
    pub(crate) idle: Duration,
}

/// The registry of session channels.
pub(crate) struct Channels {
    supervisor: Arc<Supervisor>,
    wait: Duration,
    inner: Mutex<Inner>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    feeds: mpsc::UnboundedSender<Feed>,
    /// Sessions being put back in their profile after a resume (`recovery.rs`), with a count of the
    /// flows doing it. They count as having no channel meanwhile.
    restoring: Mutex<HashMap<String, usize>>,
    /// Notifications or callbacks for a session that another channel holds (dropped; must stay 0).
    pub(crate) foreign: AtomicU64,
    /// open callbacks of a session no channel holds, by callback id: the channel id they arrived on. A
    /// subagent's card names the child session but arrives on its parent's channel, and Vibe answers it only there
    /// (`_callback_respond`: the child must belong to the channel's attached session); the lobby refuses it
    /// with "Start, resume, or continue a session". Ids are per server generation: `reset` clears them.
    guest_callbacks: Mutex<HashMap<String, u64>>,
    /// adds the session's MCP server (with its credential) to every session-opening request,
    /// after the gate. `None` when the agent tools are off.
    agent_tools: Option<Arc<crate::agent_tools::AgentTools>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The id of the session a result carries, if any.
fn result_session(result: &Value) -> Option<&str> {
    result.pointer("/state/session/id").and_then(Value::as_str)
}

/// Maps a supervisor failure to the transport's error: no ready server is `Timeout { id: 0 }`, which
/// `CoreError::from_rpc` reports as `serverUnavailable` (same rule as the lobby's).
fn to_rpc(method: &str, wait: Duration, error: SupervisorError) -> RpcError {
    match error {
        SupervisorError::Rpc(error) => error,
        SupervisorError::Handshake { source, .. } => *source,
        _ => RpcError::Timeout {
            method: method.to_owned(),
            id: 0,
            after: wait,
        },
    }
}

impl Channels {
    pub(crate) fn new(
        supervisor: Arc<Supervisor>,
        wait: Duration,
        feeds: mpsc::UnboundedSender<Feed>,
        agent_tools: Option<Arc<crate::agent_tools::AgentTools>>,
    ) -> Self {
        Self {
            supervisor,
            wait,
            inner: Mutex::new(Inner::default()),
            locks: Mutex::new(HashMap::new()),
            feeds,
            restoring: Mutex::new(HashMap::new()),
            foreign: AtomicU64::new(0),
            guest_callbacks: Mutex::new(HashMap::new()),
            agent_tools,
        }
    }

    /// Sends one gated request on the channel `route` picks (see the module doc). A session-opening request
    /// gets the session's MCP server first (the core adds it after the gate, which refuses a
    /// caller's own `mcpServers`), and its credential is bound to the session the answer names.
    pub(crate) async fn request(&self, method: &str, mut params: Value) -> Result<Value, RpcError> {
        let ticket = self
            .agent_tools
            .as_ref()
            .and_then(|tools| tools.inject(method, &mut params));
        let result = self.request_routed(method, params).await;
        if let (Some(tools), Some(ticket)) = (&self.agent_tools, ticket) {
            tools.settle(ticket, result.as_ref().ok());
        }
        result
    }

    async fn request_routed(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        match route(method, &params) {
            Route::Lobby => self.lobby(method).await?.request(method, params).await,
            Route::New => self.on_new_channel(method, params).await,
            Route::Resume(id) => self.resume(&id, params).await,
            Route::Delete(id) => {
                self.close(&id).await;
                self.lobby(method).await?.request(method, params).await
            }
            Route::Session(id) => {
                let Some(conn) = self.bound(&id) else {
                    // A subagent's card is answered on the channel it arrived on (its parent's).
                    if let Some(conn) = (method == "callback/result")
                        .then(|| params.pointer("/result/callbackId").and_then(Value::as_str))
                        .flatten()
                        .and_then(|callback| self.guest_channel(callback))
                    {
                        return conn.request(method, params).await;
                    }
                    return self.lobby(method).await?.request(method, params).await;
                };
                let hand_off = may_hand_off(method, &params);
                let result = conn.request(method, params).await?;
                if let Some(new_id) = result_session(&result).filter(|new| hand_off && *new != id) {
                    self.rebind(&id, new_id);
                }
                Ok(result)
            }
        }
    }

    async fn lobby(&self, method: &str) -> Result<Connection, RpcError> {
        self.supervisor
            .connection(self.wait)
            .await
            .map_err(|e| to_rpc(method, self.wait, e))
    }

    /// Opens a channel, subscribes the pump to it, and returns it with its key.
    async fn open(&self, method: &str) -> Result<(Connection, ChannelKey), RpcError> {
        let opened = self
            .supervisor
            .open_channel(self.wait)
            .await
            .map_err(|e| to_rpc(method, self.wait, e))?;
        let channel_id = opened.conn.channel_id().unwrap_or_default();
        let key = (opened.generation, channel_id);
        let sub = opened.conn.subscribe();
        // The pump outlives every request; a send error means the core is shutting down.
        let _ = self.feeds.send(Feed { key, sub });
        debug!(
            channel_id,
            generation = key.0,
            method,
            "channel opened for a session"
        );
        Ok((opened.conn, key))
    }

    /// `session/start` / `session/continue` on a fresh channel.
    async fn on_new_channel(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let (conn, key) = self.open(method).await?;
        match conn.request(method, params).await {
            Ok(result) => {
                match result_session(&result) {
                    Some(id) => self.bind(id, conn, key),
                    None => conn.close_input().await,
                }
                Ok(result)
            }
            Err(error) => {
                conn.close_input().await;
                Err(error)
            }
        }
    }

    /// `session/resume`: on the session's channel, or on a new one bound on success. Uses the
    /// supervisor's resume (progress while the harness holds the session lock after a crash).
    async fn resume(&self, id: &str, params: Value) -> Result<Value, RpcError> {
        let lock = self.session_lock(id);
        let _held = lock.lock().await;
        let method = "session/resume";
        if let Some(conn) = self.bound(id) {
            return self
                .supervisor
                .resume_on(&conn, id, params)
                .await
                .map_err(|e| to_rpc(method, self.wait, e));
        }
        let (conn, key) = self.open(method).await?;
        match self.supervisor.resume_on(&conn, id, params).await {
            Ok(result) => {
                self.bind(id, conn, key);
                Ok(result)
            }
            Err(error) => {
                conn.close_input().await;
                Err(to_rpc(method, self.wait, error))
            }
        }
    }

    /// Closes the session's channel (if any) and waits for the server's acknowledgement, at most
    /// 10 s: the harness flushes and releases the session lease, so the Vibe CLI can take it. Ends any
    /// work the session still had on it (Vibe ends the turn of a closed channel).
    pub(crate) async fn close(&self, id: &str) -> bool {
        self.close_matching(id, None).await
    }

    /// Close only the channel that owned an earlier request. A Stop followed by a new send may
    /// replace it before the old request reports failure; that failure must not close the new turn.
    pub(crate) async fn close_if(&self, id: &str, key: ChannelKey) -> bool {
        self.close_matching(id, Some(key)).await
    }

    async fn close_matching(&self, id: &str, expected: Option<ChannelKey>) -> bool {
        let lock = self.session_lock(id);
        let held = lock.lock().await;
        let slot = {
            let mut inner = locked(&self.inner);
            if expected.is_some_and(|key| inner.slots.get(id).is_none_or(|slot| slot.key != key)) {
                None
            } else {
                inner.close_when_quiet.remove(id);
                inner.slots.remove(id)
            }
        };
        let Some(slot) = slot else {
            drop(held);
            self.forget_lock(id, &lock);
            return false;
        };
        slot.conn.close_input().await;
        if tokio::time::timeout(Duration::from_secs(10), slot.conn.closed())
            .await
            .is_err()
        {
            warn!(
                channel_id = slot.key.1,
                "a closed channel was not acknowledged within 10 s"
            );
        }
        debug!(channel_id = slot.key.1, "session channel closed");
        drop(held);
        self.forget_lock(id, &lock);
        true
    }

    fn session_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        locked(&self.locks)
            .entry(id.to_owned())
            .or_default()
            .clone()
    }

    /// Drops the lock entry of a session nobody else is waiting on.
    fn forget_lock(&self, id: &str, lock: &Arc<tokio::sync::Mutex<()>>) {
        let mut locks = locked(&self.locks);
        // Two references: the map's and ours. Anyone else holding one is about to use it.
        if Arc::strong_count(lock) == 2 {
            locks.remove(id);
        }
    }

    fn bind(&self, id: &str, conn: Connection, key: ChannelKey) {
        let mut inner = locked(&self.inner);
        if let Some(old) = inner.slots.get(id).filter(|old| old.alive()) {
            // Never expected (opens are serialized per session); keep the newer, report it.
            warn!(
                old = old.key.1,
                new = key.1,
                "a session got a second channel; keeping the newer"
            );
        }
        inner.slots.insert(
            id.to_owned(),
            Slot {
                conn,
                key,
                last_used: Instant::now(),
            },
        );
    }

    /// Moves a channel from `from` to `to` (the server handed the channel's session to a new id). A no-op when
    /// `from` has no channel (already moved: `handoff` calls it again for every hand-off).
    pub(crate) fn rebind(&self, from: &str, to: &str) {
        let mut inner = locked(&self.inner);
        if let Some(slot) = inner.slots.remove(from) {
            debug!(
                channel_id = slot.key.1,
                "channel now carries a new session id"
            );
            inner.slots.insert(to.to_owned(), slot);
        }
    }

    /// The session's live channel, marked as used now. A binding whose channel is dead (its server
    /// died and the restart has not been handled yet) counts as no channel: a command then opens a
    /// new one on the new server instead of failing on the dead one.
    pub(crate) fn bound(&self, id: &str) -> Option<Connection> {
        let mut inner = locked(&self.inner);
        let slot = inner.slots.get_mut(id).filter(|slot| slot.alive())?;
        slot.last_used = Instant::now();
        Some(slot.conn.clone())
    }

    /// True when the session has a live channel and is not being restored (see `recovery.rs`: a
    /// command for a session being restored must wait for its profile, so it opens it itself).
    pub(crate) fn has_channel(&self, id: &str) -> bool {
        !self.is_restoring(id) && locked(&self.inner).slots.get(id).is_some_and(Slot::alive)
    }

    /// Starts one restore of `id` (counted: concurrent restores overlap).
    pub(crate) fn begin_restore(&self, id: &str) {
        *locked(&self.restoring).entry(id.to_owned()).or_insert(0) += 1;
    }

    /// Ends one restore of `id`.
    pub(crate) fn end_restore(&self, id: &str) {
        let mut restoring = locked(&self.restoring);
        if let Some(count) = restoring.get_mut(id) {
            *count -= 1;
            if *count == 0 {
                restoring.remove(id);
            }
        }
    }

    /// True while some flow is restoring `id`'s profile.
    pub(crate) fn is_restoring(&self, id: &str) -> bool {
        locked(&self.restoring).contains_key(id)
    }

    /// The session a channel carries.
    pub(crate) fn session_of(&self, key: ChannelKey) -> Option<String> {
        locked(&self.inner)
            .slots
            .iter()
            .find(|(_, slot)| slot.key == key)
            .map(|(id, _)| id.clone())
    }

    /// The live channel that carries `id`.
    pub(crate) fn key_of(&self, id: &str) -> Option<ChannelKey> {
        locked(&self.inner)
            .slots
            .get(id)
            .filter(|slot| slot.alive())
            .map(|slot| slot.key)
    }

    /// Marks the channel `key` as used (an event arrived on it).
    pub(crate) fn touch(&self, key: ChannelKey) {
        if let Some(slot) = locked(&self.inner)
            .slots
            .values_mut()
            .find(|slot| slot.key == key)
        {
            slot.last_used = Instant::now();
        }
    }

    /// A fresh subscription to channel `key` (after its old one lagged), for the pump.
    pub(crate) fn resubscribe(&self, key: ChannelKey) -> Option<Subscription> {
        locked(&self.inner)
            .slots
            .values()
            .find(|slot| slot.key == key && slot.conn.death().is_none())
            .map(|slot| slot.conn.subscribe())
    }

    /// The channel `key` ended: when it ended alone (the server closed it, or it wedged) its
    /// binding is forgotten and the session it carried is returned. When the whole process died the
    /// binding stays, so the restart (`reset`) reopens the session on the new server.
    pub(crate) fn lost(&self, key: ChannelKey) -> Option<String> {
        let mut inner = locked(&self.inner);
        let (id, slot) = inner.slots.iter().find(|(_, slot)| slot.key == key)?;
        let alone = slot.conn.death().is_some_and(|report| {
            matches!(
                report.cause,
                DeathCause::ChannelClosed { .. } | DeathCause::Wedged { .. }
            )
        });
        if !alone {
            return None;
        }
        let id = id.clone();
        inner.slots.remove(&id);
        Some(id)
    }

    /// A callback for `session_id` arrived on channel `channel_id`; when no channel holds that session (a
    /// subagent's card on its parent's channel) its answer must go back there (see `guest_callbacks`).
    /// Returns the session that channel carries (the subagent's parent) when the callback was recorded.
    pub(crate) fn note_callback(
        &self,
        session_id: &str,
        callback_id: &str,
        channel_id: u64,
    ) -> Option<String> {
        if self.key_of(session_id).is_some() {
            return None;
        }
        locked(&self.guest_callbacks).insert(callback_id.to_owned(), channel_id);
        locked(&self.inner)
            .slots
            .iter()
            .find(|(_, slot)| slot.key.1 == channel_id && slot.alive())
            .map(|(id, _)| id.clone())
    }

    /// The server closed `callback_id` (answered, cancelled, its turn ended).
    pub(crate) fn forget_callback(&self, callback_id: &str) {
        locked(&self.guest_callbacks).remove(callback_id);
    }

    /// The live channel a guest callback arrived on, marked as used now.
    fn guest_channel(&self, callback_id: &str) -> Option<Connection> {
        let channel_id = *locked(&self.guest_callbacks).get(callback_id)?;
        let mut inner = locked(&self.inner);
        let slot = inner
            .slots
            .values_mut()
            .find(|slot| slot.key.1 == channel_id && slot.alive())?;
        slot.last_used = Instant::now();
        Some(slot.conn.clone())
    }

    /// A new server generation: every binding of an older one is dead. Returns the sessions that
    /// had a channel (to be resumed on the new server).
    pub(crate) fn reset(&self, generation: u64) -> Vec<String> {
        // guest callbacks died with the old server (and channel ids start again).
        locked(&self.guest_callbacks).clear();
        let mut inner = locked(&self.inner);
        let gone: Vec<String> = inner
            .slots
            .iter()
            .filter(|(_, slot)| slot.key.0 < generation)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &gone {
            inner.slots.remove(id);
        }
        gone
    }

    /// Records the session `origin`'s main view shows (`None`: none).
    pub(crate) fn set_viewed(&self, origin: &'static str, id: Option<String>) {
        let mut inner = locked(&self.inner);
        match id {
            Some(id) => inner.viewed.insert(origin, id),
            None => inner.viewed.remove(origin),
        };
    }

    /// The sessions the windows show now (their holder is checked).
    pub(crate) fn viewed_ids(&self) -> Vec<String> {
        let inner = locked(&self.inner);
        let mut ids: Vec<String> = inner.viewed.values().cloned().collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Closes the session's channel as soon as it is quiet (archived sessions).
    pub(crate) fn close_when_quiet(&self, id: &str) {
        if self.has_channel(id) {
            locked(&self.inner).close_when_quiet.insert(id.to_owned());
        }
    }

    /// Sessions whose channel may close now: not viewed, and unused for `idle` (or archived), as
    /// judged by `quiet` (no running turn, no open callback, ...).
    pub(crate) fn idle(&self, idle: Duration, quiet: impl Fn(&str) -> bool) -> Vec<String> {
        let candidates: Vec<String> = {
            let inner = locked(&self.inner);
            let viewed: BTreeSet<&String> = inner.viewed.values().collect();
            inner
                .slots
                .iter()
                .filter(|(id, slot)| {
                    slot.alive()
                        && !viewed.contains(id)
                        && (inner.close_when_quiet.contains(*id)
                            || slot.last_used.elapsed() >= idle)
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        // `quiet` reads the store and the gate: called without the registry lock held.
        candidates.into_iter().filter(|id| quiet(id)).collect()
    }

    /// The open channels, for the status report.
    pub(crate) fn open_channels(&self) -> Vec<OpenChannel> {
        let mut open: Vec<OpenChannel> = locked(&self.inner)
            .slots
            .iter()
            .filter(|(_, slot)| slot.alive())
            .map(|(id, slot)| OpenChannel {
                session_id: id.clone(),
                channel_id: slot.key.1,
                idle: slot.last_used.elapsed(),
            })
            .collect();
        open.sort_by_key(|c| c.channel_id);
        open
    }

    /// Counts a dropped item for a session that another channel holds.
    pub(crate) fn note_foreign(&self) -> u64 {
        self.foreign.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::mpsc;
    use vw_rpc::{ConnectionBuilder, ConnectionConfig};
    use vw_supervisor::{NoSecret, Supervisor, SupervisorConfig};

    use super::Channels;

    #[tokio::test]
    async fn late_failure_cannot_close_replacement_channel() {
        let path = std::env::current_exe().unwrap();
        let config = SupervisorConfig::new(path.clone(), path.clone(), path);
        let supervisor = Arc::new(Supervisor::start_pending(
            config,
            Arc::new(NoSecret),
            "unit test".to_owned(),
        ));
        let (feeds, _received) = mpsc::unbounded_channel();
        let channels = Channels::new(supervisor, Duration::from_millis(10), feeds, None);
        let (stdout, _stdout_peer) = tokio::io::duplex(1024);
        let (stdin, _stdin_peer) = tokio::io::duplex(1024);
        let conn = ConnectionBuilder::new(ConnectionConfig::default()).start(stdout, stdin);
        channels.bind("session", conn, (2, 9));

        assert!(!channels.close_if("session", (1, 8)).await);
        assert_eq!(channels.key_of("session"), Some((2, 9)));
    }
}
