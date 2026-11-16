//! Core spawns this notification pump and callback handler: store updates, resyncs, callbacks and idle channels.
//! Uses vw-rpc/session/gate, Shared, channels, events, views and recovery; lobby leases go to `session_holder`.
//! Invariants:
//! - One pump holds a Weak to the core; one forwarder per channel feeds its bounded queue.
//! - Events naming a session held by ANOTHER channel are dropped and counted (`Channels::foreign`).
//!   Callbacks for a session with no channel (a subagent's card on its parent's channel) are kept;
//!   answers return on the arriving channel (`Channels::note_callback`).
//! - At most one resync per session runs; concurrent and deferred requests coalesce into one more read.
//! - Closed callbacks leave the gate's book; server restart or channel death closes that channel's callbacks.
//! - A lagged/gapped channel resyncs its session; a lagged/gapped lobby resyncs every loaded session.
//! - `one_ask` approves a bash callback once when its guard was already approved; other cards are remembered.
//! - `ask::grants` approves matching session grants once without another card. Grants end with the session,
//!   follow hand-offs and survive a server restart.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tracing::{debug, warn};
use vw_rpc::{Inbound, Subscription};
use vw_session::{Action, Applied, Change};
use vw_supervisor::SupervisorState;

use crate::channels::{ChannelKey, Feed};
use crate::core::Shared;
use crate::event::CoreEvent;
use crate::recovery;
use crate::resync_budget::ResyncBudget;
use crate::view::ServerStatus;

#[path = "pump_warnings.rs"]
mod warnings;

#[path = "pump_resync.rs"]
mod resync;
use resync::next_deferred;
pub(crate) use resync::refresh;

#[cfg(test)]
#[path = "pump_resync_tests.rs"]
mod resync_tests;

#[cfg(test)]
#[path = "pump_browser_tests.rs"]
mod browser_tests;

#[path = "pump_browser.rs"]
mod browser;

#[path = "pump_callbacks.rs"]
mod callbacks;
pub(crate) use callbacks::callback_handler;

/// Items of the lobby or of one session channel, as the forwarders deliver them. `key` is `None`
/// for the lobby.
struct Item {
    key: Option<ChannelKey>,
    inbound: Option<Inbound>,
    lagged: bool,
}

/// What one finished resync reports back.
struct Resynced {
    session_id: String,
    applied: Option<Applied>,
}

/// Mutable state of the pump loop.
struct Pump {
    weak: Weak<Shared>,
    generation: u64,
    items: mpsc::Sender<Item>,
    resyncs: JoinSet<Resynced>,
    /// Browser releases after terminal turn updates; cancelled when the pump ends.
    browser_releases: JoinSet<()>,
    /// One owned deadline per deferred request (store delay or budget expiry), never periodic reads.
    later: BTreeMap<String, tokio::time::Instant>,
    in_flight: BTreeSet<String>,
    queued: BTreeSet<String>,
    open_callbacks: BTreeMap<String, Vec<String>>,
    budget: ResyncBudget,
}

/// Forwards one subscription into the pump's queue until it ends.
fn forward(sub: Subscription, key: Option<ChannelKey>, items: mpsc::Sender<Item>) {
    tokio::spawn(async move {
        let mut sub = sub;
        loop {
            let inbound = sub.recv().await;
            let end = inbound.is_none();
            let item = Item {
                key,
                lagged: end && sub.was_dropped_for_lag(),
                inbound,
            };
            if items.send(item).await.is_err() || end {
                return;
            }
        }
    });
}

/// Runs until the core is dropped or the supervisor is gone. `first` is the lobby subscription,
/// `loads` carries the store updates that commands made, `feeds` the subscriptions of new channels.
pub(crate) async fn run(
    weak: Weak<Shared>,
    first: Option<Subscription>,
    mut loads: mpsc::Receiver<Applied>,
    mut feeds: mpsc::UnboundedReceiver<Feed>,
) {
    let Some((mut states, idle_check)) = weak
        .upgrade()
        .map(|s| (s.supervisor.state(), s.config.idle_check()))
    else {
        return;
    };
    let generation = match &*states.borrow_and_update() {
        SupervisorState::Ready { generation, .. } => *generation,
        _ => 0,
    };
    let (items, mut queue) = mpsc::channel(4096);
    if let Some(sub) = first {
        forward(sub, None, items.clone());
    }
    let mut pump = Pump {
        weak,
        generation,
        items,
        resyncs: JoinSet::new(),
        browser_releases: JoinSet::new(),
        later: BTreeMap::new(),
        in_flight: BTreeSet::new(),
        queued: BTreeSet::new(),
        open_callbacks: BTreeMap::new(),
        budget: ResyncBudget::standard(),
    };
    let mut idle = tokio::time::interval(idle_check);
    idle.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            Some(item) = queue.recv() => {
                if !pump.inbound(item) {
                    return;
                }
            }
            Some(feed) = feeds.recv() => forward(feed.sub, Some(feed.key), pump.items.clone()),
            changed = states.changed() => {
                if changed.is_err() {
                    return;
                }
                let state = states.borrow_and_update().clone();
                if !pump.server_state(&state) {
                    return;
                }
            }
            Some(applied) = loads.recv() => {
                let Some(shared) = pump.weak.upgrade() else {
                    return;
                };
                pump.after_apply(&shared, &applied);
            }
            Some(_) = pump.browser_releases.join_next(), if !pump.browser_releases.is_empty() => {}
            id = next_deferred(&pump.later) => {
                pump.later.remove(&id);
                let Some(shared) = pump.weak.upgrade() else {
                    return;
                };
                pump.resync(&shared, &id);
            }
            Some(done) = pump.resyncs.join_next(), if !pump.resyncs.is_empty() => {
                if let Ok(done) = done
                    && !pump.resynced(&done) {
                        return;
                    }
            }
            _ = idle.tick() => {
                let Some(shared) = pump.weak.upgrade() else {
                    return;
                };
                recovery::close_idle(&shared);
                // archived sessions that became quiet end their resources now.
                crate::session_end::run_pending(&shared);
            }
        }
    }
}

impl Pump {
    /// Handles one item of the lobby or a channel. Returns false when the core is gone.
    fn inbound(&mut self, item: Item) -> bool {
        let Some(shared) = self.weak.upgrade() else {
            return false;
        };
        if let Some(key) = item.key {
            // A channel of a dead server. (A newer one is fine: a command can open a channel on the
            // new server before this loop has seen the generation change.)
            if key.0 < self.generation {
                return true;
            }
            shared.channels.touch(key);
        }
        match item.inbound {
            Some(Inbound::Notification(mut n)) => {
                // Names and ids only (never params): enough to reconstruct an event order.
                let session = n.params.get("sessionId").and_then(Value::as_str);
                let event_id = n.params.get("eventId").and_then(Value::as_u64);
                debug!(method = %n.method, session, event_id, channel = ?item.key, "notification");
                if let (Some(sid), Some(key)) = (session, item.key)
                    && shared
                        .channels
                        .key_of(sid)
                        .is_some_and(|owner| owner != key)
                {
                    let total = shared.channels.note_foreign();
                    warn!(
                        channel = key.1,
                        total, "event for a session another channel carries; dropped"
                    );
                    return true;
                }
                if n.method == "session/lease/updated" && item.key.is_none() {
                    crate::session_holder::lease_updated(&shared, session);
                }
                crate::resume_models::notification(&shared, &n.method, &n.params);
                if matches!(n.method.as_str(), "turn/started" | "turn/completed")
                    && let Some(sid) = session
                {
                    shared.interrupted_send.turn_event(sid, &n.params["turn"]);
                }
                if n.method == "runtime/updated" {
                    shared.gate.observe_runtime(&n.params);
                    // Not while the core restores the session's profile: right after a resume Vibe
                    // reports the profile the resume ran with, which must not replace what the session had.
                    if let Some(sid) = session.filter(|sid| !shared.channels.is_restoring(sid)) {
                        let previous_mode = crate::chrome_control::check::mode(&shared, sid);
                        shared.access.observe_runtime(sid, &n.params["runtime"]);
                        crate::chrome_control::check::mode_changed(
                            &shared,
                            sid,
                            previous_mode.as_deref(),
                        );
                    }
                }
                // Before any projection changes the entry: the receipt as Vibe reported it on this channel.
                crate::ask::tool_calls::receipts::observe(&shared, item.key, &n.method, &n.params);
                crate::agent_tools::shell_commands::project(&shared, &mut n.params);
                #[cfg(feature = "video")]
                crate::video_projection::apply(&shared, &mut n.params);
                crate::edit_preview::project(&shared, &mut n.params);
                crate::chrome_control::approval::project(&shared, &mut n.params);
                crate::paused_tools_gate::mark_held_calls(&shared, &mut n.params);
                // A title or preview change (an auto-title, a rename) holds no vault value from here on.
                crate::title_guard::withhold_in(&shared, &mut n.params);
                let session = item.key.and_then(|key| shared.channels.session_of(key));
                let applied = warnings::apply(&shared.store, n, session);
                self.after_apply(&shared, &applied);
            }
            Some(Inbound::Gap(reason)) => {
                warn!(?reason, channel = ?item.key, "protocol data lost; resyncing");
                self.resync_source(&shared, item.key);
            }
            None => self.ended(&shared, item.key, item.lagged),
        }
        true
    }
    /// A subscription ended: lagged (resubscribe, resync) or its channel died.
    fn ended(&mut self, shared: &Arc<Shared>, key: Option<ChannelKey>, lagged: bool) {
        match (key, lagged) {
            (None, true) => {
                warn!("lobby subscription lagged; resubscribing and resyncing");
                if let Some(conn) = shared.supervisor.current() {
                    forward(conn.subscribe(), None, self.items.clone());
                }
                self.resync_all(shared);
            }
            (None, false) => {} // the lobby died: a restart (new generation) follows
            (Some(key), true) => {
                warn!(
                    channel = key.1,
                    "channel subscription lagged; resubscribing"
                );
                if let Some(sub) = shared.channels.resubscribe(key) {
                    forward(sub, Some(key), self.items.clone());
                }
                self.resync_source(shared, Some(key));
            }
            (Some(key), false) => {
                // Our own closes unbind first; a binding still here means the channel died on its
                // own (closed by the server, or wedged). Its callbacks died with it; re-read the
                // session from the lobby to show what Vibe stored.
                if let Some(id) = shared.channels.lost(key) {
                    warn!(channel = key.1, "a session channel ended on its own");
                    shared.asks.cancel_session(Some(&id), "the session closed");
                    shared.gate.session_closed(&id);
                    self.open_callbacks.remove(&id);
                    self.resync(shared, &id);
                }
                crate::keep_awake::reconcile(shared);
            }
        }
    }

    /// Resyncs the session of channel `key`, or every loaded session for the lobby.
    fn resync_source(&mut self, shared: &Arc<Shared>, key: Option<ChannelKey>) {
        match key {
            None => self.resync_all(shared),
            Some(key) => {
                if let Some(id) = shared.channels.session_of(key) {
                    self.resync(shared, &id);
                }
            }
        }
    }

    /// Follows the supervisor. A new generation means a new server: subscribe to its lobby, close
    /// the callbacks of the old one, and read the sessions whose channels died, leaving them detached.
    /// Returns false when the core is gone.
    fn server_state(&mut self, state: &SupervisorState) -> bool {
        let Some(shared) = self.weak.upgrade() else {
            return false;
        };
        if !matches!(state, SupervisorState::Ready { .. }) {
            shared.keep_awake.working(false);
        }
        shared.emit(CoreEvent::Server {
            status: ServerStatus::from(state),
        });
        if let SupervisorState::Ready { generation, .. } = state
            && *generation != self.generation
        {
            self.generation = *generation;
            self.later.clear();
            self.queued.clear();
            self.budget = ResyncBudget::standard();
            for view in shared.store.views() {
                shared
                    .asks
                    .cancel_session(Some(&view.session_id), "Vibe restarted");
                shared.gate.session_closed(&view.session_id);
            }
            self.open_callbacks.clear();
            if let Some(conn) = shared.supervisor.current() {
                forward(conn.subscribe(), None, self.items.clone());
            }
            let reopen = shared.channels.reset(*generation);
            debug!(
                generation,
                reopen = reopen.len(),
                "pump follows a new server"
            );
            let inflight = recovery::in_progress_turns(&shared, reopen);
            recovery::restore_detached(&shared, inflight);
        }
        true
    }

    /// Carries out the actions of one store update and keeps the callback book in step.
    fn after_apply(&mut self, shared: &Arc<Shared>, applied: &Applied) {
        for change in &applied.changes {
            if let Change::Replaced { session_id } = change {
                shared.history_loading.start(shared, session_id);
            }
            if let Change::Removed { session_id } = change {
                shared.history_loading.stop(session_id);
                self.later.remove(session_id);
                self.queued.remove(session_id);
                self.budget.forget(session_id);
            }
            crate::guard_refusals::observe(shared, change);
            self.browser_changed(shared, change);
            crate::on_demand::observe(shared, change);
            crate::queue_return::observe(shared, change);
            crate::stops::observe(shared, change);
            crate::resume_models::observe(shared, change);
            if let Change::CallbacksChanged {
                session_id,
                open_callback_ids,
            } = change
            {
                if let Some(hub) = &shared.preview {
                    hub.service
                        .browser_turn_waiting(session_id, !open_callback_ids.is_empty());
                }
                let before = self
                    .open_callbacks
                    .insert(session_id.clone(), open_callback_ids.clone())
                    .unwrap_or_default();
                for closed in before.iter().filter(|id| !open_callback_ids.contains(id)) {
                    crate::ask::callback_closed(shared, closed);
                    shared.gate.callback_closed(closed);
                    shared.channels.forget_callback(closed);
                }
            }
            if let Change::Removed { session_id } = change {
                self.open_callbacks.remove(session_id);
                shared
                    .asks
                    .cancel_session(Some(session_id), "the session closed");
                shared.gate.session_closed(session_id);
                crate::ask::grants::session_ended(shared, session_id);
            }
        }
        if applied.changes.iter().any(|change| {
            matches!(
                change,
                Change::Replaced { .. }
                    | Change::Removed { .. }
                    | Change::SessionPatched { .. }
                    | Change::TurnUpserted { .. }
                    | Change::ChildUpserted { .. }
                    | Change::CallbacksChanged { .. }
            )
        }) {
            crate::keep_awake::reconcile(shared);
        }
        for action in &applied.actions {
            let Action::NeedsResync {
                session_id,
                reason,
                delayed,
            } = action;
            debug!(session_id, reason, delayed, "store asked for a resync");
            if *delayed {
                self.later
                    .entry(session_id.clone())
                    .or_insert_with(|| tokio::time::Instant::now() + DELAYED_RESYNC);
            } else {
                self.resync(shared, session_id);
            }
        }
    }
}

/// How long a delayed re-read waits (the safety net after a read that could not settle a held event).
pub(crate) const DELAYED_RESYNC: Duration = Duration::from_secs(3);

/// Lets the pump's idle tick be at most this coarse.
pub(crate) const MAX_IDLE_CHECK: Duration = Duration::from_secs(30);
