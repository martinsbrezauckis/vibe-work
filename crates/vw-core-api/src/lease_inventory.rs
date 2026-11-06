//! One lease inventory stream per server generation, shared by every holder query.
//! Uses a dedicated initialized channel so snapshot-before-notifications ordering is unambiguous, including
//! reconnects. The holder poll owns retry; loss preserves published holders until a fresh snapshot arrives.

use crate::core::Shared;
use crate::{CoreError, ErrorCode};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use vw_rpc::Inbound;
use vw_supervisor::SupervisorState;

#[derive(Default)]
struct State {
    serial: u64,
    active: Option<(u64, u64)>,
    held: BTreeMap<String, u32>,
    retry: Option<(Instant, CoreError)>,
    retry_generation: Option<u64>,
    task: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub(super) struct Inventory {
    state: Mutex<State>,
    connecting: tokio::sync::Mutex<()>,
    stopped: AtomicBool,
}

fn generation(shared: &Shared) -> Option<u64> {
    match *shared.supervisor.state().borrow() {
        SupervisorState::Ready { generation, .. } => Some(generation),
        _ => None,
    }
}

impl Inventory {
    pub(super) fn retry_at(&self) -> Option<Instant> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retry
            .as_ref()
            .map(|(at, _)| *at)
    }

    pub(super) fn holder(&self, id: &str) -> Option<u32> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .held
            .get(id)
            .copied()
    }

    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active = None;
        if let Some(task) = state.task.take() {
            task.abort();
        }
    }

    fn lost(&self, token: (u64, u64)) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.active == Some(token) {
            state.active = None;
        }
    }

    fn update(&self, token: (u64, u64), params: &Value) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.active != Some(token) {
            return false;
        }
        if let Some(id) = params["sessionId"].as_str() {
            if let Some(pid) = holder(params) {
                state.held.insert(id.to_owned(), pid);
            } else {
                state.held.remove(id);
            }
            return true;
        }
        false
    }
}

impl Drop for Inventory {
    fn drop(&mut self) {
        self.stop();
    }
}

/// All concurrent readers share this bounded initial read/reconnect, including failed attempts.
pub(super) async fn ensure(shared: &Arc<Shared>) -> Result<(), CoreError> {
    let inventory = &shared.holders.inventory;
    let _connecting = inventory.connecting.lock().await;
    if inventory.stopped.load(Ordering::Relaxed) {
        return Err(CoreError::new(
            ErrorCode::Cancelled,
            "session holder tracking stopped",
        ));
    }
    let current = generation(shared);
    {
        let state = inventory
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.active.is_some_and(|(at, _)| Some(at) == current) {
            return Ok(());
        }
        if let Some((until, error)) = &state.retry
            && *until > Instant::now()
            && state.retry_generation == current
        {
            return Err(error.clone());
        }
    }
    let result = if let Some(current) = current {
        tokio::time::timeout(super::READ_WAIT, connect(shared, current))
            .await
            .map_err(|_| {
                CoreError::new(ErrorCode::Timeout, "the session's holder could not be read")
            })
            .and_then(|result| result)
    } else {
        Err(CoreError::new(
            ErrorCode::ServerUnavailable,
            "Vibe is not running",
        ))
    };
    if let Err(error) = &result {
        let mut state = inventory
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.retry = Some((Instant::now() + super::POLL, error.clone()));
        state.retry_generation = current;
    }
    result
}

async fn connect(shared: &Arc<Shared>, current: u64) -> Result<(), CoreError> {
    let states = shared.supervisor.state();
    let channel = shared
        .supervisor
        .open_channel(super::READ_WAIT)
        .await
        .map_err(|error| CoreError::new(ErrorCode::ServerUnavailable, error.to_string()))?;
    let events = channel.conn.subscribe();
    let snapshot = channel
        .conn
        .request("session/lease/subscribe", json!({}))
        .await
        .map_err(|error| CoreError::from_rpc("session/lease/subscribe", &error))?;
    let rows = snapshot["held"]
        .as_array()
        .filter(|rows| rows.iter().all(|row| row["sessionId"].is_string()))
        .ok_or_else(|| {
            CoreError::new(
                ErrorCode::ServerError,
                "Vibe returned an invalid session holder inventory",
            )
        })?;
    if channel.generation != current || generation(shared) != Some(current) {
        return Err(CoreError::new(
            ErrorCode::ServerUnavailable,
            "Vibe restarted while reading session holders",
        ));
    }
    let weak = Arc::downgrade(shared);
    let mut state = shared
        .holders
        .inventory
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if shared.holders.inventory.stopped.load(Ordering::Relaxed) {
        return Err(CoreError::new(
            ErrorCode::Cancelled,
            "session holder tracking stopped",
        ));
    }
    if let Some(task) = state.task.take() {
        task.abort();
    }
    state.serial += 1;
    let token = (current, state.serial);
    state.active = Some(token);
    state.retry = None;
    state.held = rows
        .iter()
        .filter_map(|row| Some((row["sessionId"].as_str()?.to_owned(), holder(row)?)))
        .collect();
    state.task = Some(tokio::spawn(follow(weak, token, states, events, channel)));
    shared.holders.changed(None);
    Ok(())
}

async fn follow(
    weak: std::sync::Weak<Shared>,
    token: (u64, u64),
    mut states: tokio::sync::watch::Receiver<SupervisorState>,
    mut events: vw_rpc::Subscription,
    channel: vw_supervisor::OpenedChannel,
) {
    loop {
        tokio::select! {
            changed = states.changed() => {
                if changed.is_err() || !matches!(*states.borrow_and_update(), SupervisorState::Ready { generation, .. } if generation == token.0) { break; }
            }
            inbound = events.recv() => {
                let Some(Inbound::Notification(event)) = inbound else { break; };
                if event.method == "session/lease/ended" { break; }
                if event.method == "session/lease/updated" {
                    let Some(shared) = weak.upgrade() else { return; };
                    if !shared.holders.inventory.update(token, &event.params) { break; }
                    shared.holders.changed(event.params["sessionId"].as_str());
                }
            }
        }
    }
    // Keep the connection alive until the stream ends, then release this unbound channel.
    drop(channel);
    if let Some(shared) = weak.upgrade() {
        shared.holders.inventory.lost(token);
        shared.holders.changed(None);
    }
}

/// A locked lease with no readable diagnostic still counts as held (pid 0).
fn holder(row: &Value) -> Option<u32> {
    let holder = row.get("leaseHolder").filter(|holder| !holder.is_null())?;
    Some(
        holder
            .get("processId")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn lost_or_ended_stream_preserves_holders_and_requires_a_fresh_inventory() {
        let core = crate::offline_core::start("lease-stream-loss").await;
        for end in [
            "not-json\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"session/lease/ended\",\"params\":{}}\n",
        ] {
            let (mut write, read) = tokio::io::duplex(1024);
            let conn = vw_rpc::ConnectionBuilder::new(vw_rpc::ConnectionConfig::default())
                .start(read, tokio::io::sink());
            let events = conn.subscribe();
            core.shared.holders.inventory.state.lock().unwrap().active = Some((1, 1));
            let worker = tokio::spawn(follow(
                Arc::downgrade(&core.shared),
                (1, 1),
                core.shared.supervisor.state(),
                events,
                vw_supervisor::OpenedChannel {
                    conn,
                    generation: 1,
                },
            ));
            write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"session/lease/updated\",\"params\":{\"sessionId\":\"a\",\"leaseHolder\":{\"processId\":42}}}\n").await.unwrap();
            write.write_all(end.as_bytes()).await.unwrap();
            tokio::time::timeout(super::super::READ_WAIT, worker)
                .await
                .unwrap()
                .unwrap();
            let state = core.shared.holders.inventory.state.lock().unwrap();
            assert_eq!(state.active, None, "loss requires a fresh subscription");
            assert_eq!(
                state.held.get("a"),
                Some(&42),
                "loss never declares a known lock free"
            );
        }
        core.shutdown().await;
    }

    #[test]
    fn stale_stream_cannot_change_or_invalidate_the_new_generation() {
        let inventory = Inventory::default();
        inventory.state.lock().unwrap().active = Some((2, 2));
        inventory.update(
            (2, 2),
            &json!({"sessionId":"a","leaseHolder":{"processId":42}}),
        );
        inventory.update((1, 1), &json!({"sessionId":"a","leaseHolder":null}));
        inventory.lost((1, 1));
        assert_eq!(inventory.holder("a"), Some(42));
        assert_eq!(inventory.state.lock().unwrap().active, Some((2, 2)));
        inventory.update((2, 2), &json!({"sessionId":"a","leaseHolder":{}}));
        assert_eq!(inventory.holder("a"), Some(0));
        inventory.update((2, 2), &json!({"sessionId":"a","leaseHolder":null}));
        assert_eq!(inventory.holder("a"), None);
        inventory.lost((2, 2));
        assert_eq!(inventory.state.lock().unwrap().active, None);
    }
}
