//! Feeds channel notifications into the store, resyncs, maintains callbacks and model aliases,
//! restores sessions after server restarts, closes idle channels and answers `callback/call`.
//! Depends on: vw-rpc (subscriptions, server requests), vw-session (store), vw-gate (callbacks,
//! aliases), `core::Shared`, `channels`, `event`, `view`, `recovery`. Called by: `core` (spawns `run`,
//! installs `callback_handler` on the supervisor).
//! Invariants:
//! - One pump task per core; it holds only a `Weak` to the core, so dropping the last `Core`
//!   handle ends it. One small forwarder task per channel subscription feeds it (bounded queue).
//! - Events of a channel belong to its session: a notification or callback naming a session that
//!   ANOTHER channel carries is dropped and counted (`Channels::foreign`; Vibe never sent one in the
//!   observed session streams). A callback naming a session NO channel holds (a subagent's card on its parent's
//!   channel) is kept, and its answer goes back on the channel it arrived on (`Channels::note_callback`).
//! - At most one `session/read` resync per session is in flight; requests that arrive meanwhile
//!   are coalesced into one more read after it (a bad stream cannot fan out reads). Nothing is read
//!   on a timer: every session with a channel streams its own events (removed the poll).
//! - When a session's open callbacks shrink, the closed ids are removed from the gate's book; when
//!   the server restarts or a session's channel dies, those callbacks are closed (they died with it).
//! - A lagged or gapped channel leads to a resync of its session; a lagged or gapped lobby to a
//!   resync of every loaded session.
//! - One approval per call (`one_ask`): an approval card for a bash call whose guard dialog the user approved moments before is
//!   answered "approve" (once) here and never shown; every other card is shown and remembered for its answer.
//! - Session grants (`ask::grants`): an approval whose tool and required permissions equal a grant the user made
//!   with "Allow for this session" earlier in the same session is answered "approve" (once) here and never shown.
//!   Grants end when their session leaves the store and follow a hand-off; a server restart keeps them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tracing::{debug, warn};
use vw_protocol::ProtocolErrorCode;
use vw_rpc::{HandlerError, Inbound, ServerRequest, Subscription};
use vw_session::{Action, Applied, Change, LoadKind, Notification};
use vw_supervisor::SupervisorState;

use crate::channels::{ChannelKey, Feed};
use crate::core::Shared;
use crate::event::{AlertKind, CoreEvent};
use crate::recovery;
use crate::resync_budget::{Budget, ResyncBudget};
use crate::view::ServerStatus;

/// The `callback/call` handler: registers the callback with the gate, tells subscribers, and acks
/// `{callbackId, accepted: true}` (the answer itself comes later as
/// `callback/result`). A callback the gate cannot register, or one that arrives on a channel other
/// than the one carrying its session, is refused with `invalid_params`. A card the user already
/// answered in a guard's dialog, or the sound-only read-aloud tool, is answered by the core
/// (`carry_answer`) instead of being shown.
pub(crate) fn callback_handler(
    slot: Arc<OnceLock<Weak<Shared>>>,
) -> impl Fn(ServerRequest) -> std::future::Ready<Result<Value, HandlerError>> + Send + Sync + 'static
{
    move |mut request: ServerRequest| {
        let Some(shared) = slot.get().and_then(Weak::upgrade) else {
            return std::future::ready(Err(HandlerError::new(
                ProtocolErrorCode::InternalError,
                "the client core is not ready",
            )));
        };
        crate::agent_tools::shell_commands::project(&shared, &mut request.params);
        if let Err(error) = crate::agent_tools::shell_commands::verified(&request.params) {
            return std::future::ready(Err(error));
        }
        let callback = request.params.get("callback");
        let text = |key: &str| {
            callback
                .and_then(|c| c.get(key))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let (callback_id, session_id) = (text("id"), text("sessionId"));
        let owner = shared.channels.key_of(&session_id).map(|key| key.1);
        if let (Some(owner), Some(arrived)) = (owner, request.channel_id)
            && owner != arrived
        {
            let total = shared.channels.note_foreign();
            warn!(
                arrived,
                owner, total, "callback for a session another channel carries; refused"
            );
            return std::future::ready(Err(HandlerError::new(
                ProtocolErrorCode::InvalidParams,
                "callback for a session of another channel",
            )));
        }
        // A subagent's card (its child session has no channel) is answered on the channel it came on, and
        // shown in the parent's view (the parent is the session that channel carries).
        let subagent = request.channel_id.and_then(|arrived| {
            shared
                .channels
                .note_callback(&session_id, &callback_id, arrived)
                .map(|parent_session_id| crate::event::SubagentCard {
                    parent_session_id,
                    entry: callback.cloned().unwrap_or(Value::Null),
                })
        });
        let outcome = match shared.gate.register_callback(&request.params) {
            Ok(kind) => {
                let entry = callback.cloned().unwrap_or(Value::Null);
                // A grant made on a subagent's card ends with the parent session that carries it.
                let owner = subagent
                    .as_ref()
                    .map_or_else(|| session_id.clone(), |s| s.parent_session_id.clone());
                if crate::stop_tree::cancel_late(&shared, &session_id, &owner)
                    || crate::ask::tool_calls::dispatch_transport(&shared, &entry, &owner)
                {
                    return std::future::ready(Ok(
                        json!({"callbackId":callback_id,"accepted":true}),
                    ));
                }
                crate::ask::callback_opened(&shared, &entry, kind.as_str(), &owner);
                let handle = tokio::runtime::Handle::try_current().ok();
                // Only with a runtime to answer on (the guard answer is consumed by the check).
                let carried = handle
                    .as_ref()
                    .and_then(|_| carried_for(&shared, &session_id, &entry, kind.as_str()));
                if let (Some(handle), Some(carried)) = (handle, carried) {
                    handle.spawn(carry_answer(
                        shared.clone(),
                        session_id,
                        callback_id.clone(),
                        carried,
                    ));
                } else {
                    // remembered so the user's answer counts for the guard inside our own tool.
                    if let Some(ledger) = crate::one_ask::of(&shared) {
                        ledger.card_opened(&entry);
                    }
                    shared.emit(CoreEvent::CallbackOpened {
                        session_id,
                        callback_id: callback_id.clone(),
                        kind: kind.as_str().to_owned(),
                        subagent,
                    });
                }
                Ok(json!({"callbackId": callback_id, "accepted": true}))
            }
            Err(fault) => {
                warn!(reason = %fault, "callback/call refused");
                Err(HandlerError::new(
                    ProtocolErrorCode::InvalidParams,
                    fault.to_string(),
                ))
            }
        };
        std::future::ready(outcome)
    }
}

/// Whether the core answers this card itself, checked in this order: a session grant ("Allow for this session"
/// earlier in this session, for this tool and exactly these permissions), the sound-only read-aloud tool, or the
/// user's guard answer for the same call (`one_ask`, which consumes it).
fn carried_for(
    shared: &Arc<Shared>,
    session_id: &str,
    entry: &Value,
    kind: &str,
) -> Option<Carried> {
    // A removed connector's card is shown instead (its tool's grants ended with it; this covers a card that arrived
    // between the pre-tool hook's check and the removal): the user's yes there becomes a refusal.
    let removed = entry["detail"]["effect"]["toolName"]
        .as_str()
        .is_some_and(|tool| shared.connectors.removed_tool(tool));
    if kind == "approval"
        && !removed
        && let Some(grant) = shared.ask_grants.matching(session_id, entry)
    {
        return Some(Carried::Grant(grant.id));
    }
    if shared.agent_tools.is_some() && crate::read_aloud::approve_without_dialog(entry) {
        return Some(Carried::ReadAloud);
    }
    if shared.agent_tools.is_some() && crate::on_demand::approve_without_dialog(entry) {
        return Some(Carried::OnDemand);
    }
    crate::one_ask::of(shared)
        .is_some_and(|l| l.answer_for_card(entry))
        .then_some(Carried::GuardAnswer)
}

/// Why the core answers an approval card itself instead of showing it.
#[derive(Clone)]
enum Carried {
    /// The user's yes in a guard's dialog for the same call (`one_ask`).
    GuardAnswer,
    /// The sound-only read-aloud tool (`read_aloud`).
    ReadAloud,
    /// A call that only turns tools on for the chat (`on_demand`): it does nothing itself.
    OnDemand,
    /// A session grant Vibe Work keeps ("Allow for this session" earlier, same tool and permissions); its id.
    Grant(String),
}

impl Carried {
    fn source(&self) -> &'static str {
        match self {
            Self::GuardAnswer => "user's guard answer",
            Self::ReadAloud => "read-aloud playback policy",
            Self::OnDemand => "turning tools on for the chat",
            Self::Grant(_) => "session grant",
        }
    }
    /// The ask record: a grant's answer is `Allowed { Session, Grant }` (the audit row "Allowed for this session
    /// (earlier answer)"); the other two answer this one call for the core.
    fn outcome(&self) -> vw_ask::Outcome {
        match self {
            Self::Grant(_) => {
                crate::ask::allowed(vw_ask::AnsweredBy::Grant, vw_ask::Scope::Session)
            }
            Self::GuardAnswer | Self::ReadAloud | Self::OnDemand => {
                crate::ask::allowed(vw_ask::AnsweredBy::Core, vw_ask::Scope::Once)
            }
        }
    }
}

/// Answers Vibe's approval card `callback_id` once (`approve`): the user's earlier guard answer, the read-aloud
/// playback policy, or a session grant. The audit row records the app origin and the ask record why. When the
/// send fails, the audit says the answer was not confirmed; a claimed callback cannot be offered again.
async fn carry_answer(
    shared: Arc<Shared>,
    session_id: String,
    callback_id: String,
    carried: Carried,
) {
    let source = carried.source();
    shared.ask_grants.forget_callback(&callback_id);
    let Some(delivery) = shared.asks.reserve(
        &format!("callback:{callback_id}"),
        match &carried {
            Carried::Grant(_) => vw_ask::AnsweredBy::Grant,
            _ => vw_ask::AnsweredBy::Core,
        },
    ) else {
        return;
    };
    match shared
        .gate
        .answer_callback(&session_id, &callback_id, crate::one_ask::approve_once())
        .await
    {
        Ok(_) => {
            delivery.confirmed(carried.outcome());
            tracing::info!(
                session = %session_id,
                callback = %callback_id,
                source,
                "approval card answered once by the app"
            );
            // the grant's use count and last use (the Allowed page and the session's grant list show them).
            if let Carried::Grant(grant) = &carried
                && shared.ask_grants.record_use(grant, shared.asks.now_ms())
            {
                shared.emit(crate::ask::grants::event(&shared));
            }
            shared.emit(CoreEvent::CallbackAnswered {
                session_id,
                callback_id,
                origin: crate::command::Origin::Ui,
            });
        }
        Err(problem) => {
            warn!(reason = %problem, callback = %callback_id, "automatic approval answer not confirmed");
        }
    }
}

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
    /// Delayed re-reads the store asked for (`Action::NeedsResync { delayed: true }`): one per session at a time,
    /// each a single resync through the budget after [`DELAYED_RESYNC`], never a loop.
    later: JoinSet<String>,
    later_ids: BTreeSet<String>,
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
        later: JoinSet::new(),
        later_ids: BTreeSet::new(),
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
            Some(Ok(id)) = pump.later.join_next(), if !pump.later.is_empty() => {
                pump.later_ids.remove(&id);
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
                if n.method == "runtime/updated" {
                    shared.gate.observe_runtime(&n.params);
                    // Not while the core restores the session's profile: right after a resume Vibe
                    // reports the profile the resume ran with, which must not replace what the session had.
                    if let Some(sid) = session.filter(|sid| !shared.channels.is_restoring(sid)) {
                        shared.access.observe_runtime(sid, &n.params["runtime"]);
                    }
                }
                crate::agent_tools::shell_commands::project(&shared, &mut n.params);
                let applied = shared.store.apply(Notification::new(n.method, n.params));
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
    /// the callbacks of the old one, and resume (on new channels) the sessions that had a channel.
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
            recovery::reopen(&shared, inflight);
        }
        true
    }

    /// Keeps browser protection in step with turn events and replacement snapshots.
    fn browser_changed(&mut self, shared: &Arc<Shared>, change: &Change) {
        if let Change::Replaced { session_id } = change
            && let Some(hub) = &shared.preview
            && let Some(view) = shared.store.view(session_id)
        {
            if view.state["turns"]
                .as_array()
                .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "in_progress"))
            {
                hub.service.browser_turn_active(session_id);
            } else if let Some(epoch) = hub.service.browser_turn_epoch(session_id) {
                let (shared, session_id) = (shared.clone(), session_id.clone());
                self.browser_releases.spawn(async move {
                    crate::browser::turn_ended(&shared, &session_id, epoch).await;
                });
            }
            hub.service.browser_turn_waiting(
                session_id,
                view.state["activeCallbacks"]
                    .as_array()
                    .is_some_and(|callbacks| !callbacks.is_empty()),
            );
        }
        if let Change::TurnUpserted { session_id, turn } = change
            && turn["status"] == "in_progress"
            && let Some(hub) = &shared.preview
        {
            hub.service.browser_turn_active(session_id);
        }
        if let Change::TurnUpserted { session_id, turn } = change
            && matches!(
                turn["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            )
            && let Some(epoch) = shared
                .preview
                .as_ref()
                .and_then(|hub| hub.service.browser_turn_epoch(session_id))
        {
            let (shared, session_id) = (shared.clone(), session_id.clone());
            self.browser_releases.spawn(async move {
                crate::browser::turn_ended(&shared, &session_id, epoch).await;
            });
        }
    }

    /// Carries out the actions of one store update and keeps the callback book in step.
    fn after_apply(&mut self, shared: &Arc<Shared>, applied: &Applied) {
        for change in &applied.changes {
            crate::guard_refusals::observe(shared, change);
            self.browser_changed(shared, change);
            crate::on_demand::observe(shared, change);
            crate::queue_return::observe(shared, change);
            crate::stops::observe(shared, change);
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
            if !*delayed {
                self.resync(shared, session_id);
            } else if self.later_ids.insert(session_id.clone()) {
                let id = session_id.clone();
                self.later.spawn(async move {
                    tokio::time::sleep(DELAYED_RESYNC).await;
                    id
                });
            }
        }
    }

    fn resync_all(&mut self, shared: &Arc<Shared>) {
        for view in shared.store.views() {
            self.resync(shared, &view.session_id);
        }
    }

    /// Starts a `session/read` for `session_id`, or queues one more if a read is in flight.
    fn resync(&mut self, shared: &Arc<Shared>, session_id: &str) {
        if self.in_flight.contains(session_id) {
            self.queued.insert(session_id.to_owned());
            return;
        }
        match self.budget.spend(session_id, std::time::Instant::now()) {
            Budget::Allowed => {}
            Budget::Paused { first } => {
                if first {
                    warn!(
                        session_id,
                        "too many resyncs; pausing re-reads of this session"
                    );
                    shared.emit(CoreEvent::Alert {
                        session_id: Some(session_id.to_owned()),
                        kind: AlertKind::ResyncPaused,
                        message: "A session's data kept changing under the app, so it stopped re-reading it for a while. What you see may be out of date.".to_owned(),
                    });
                }
                return;
            }
        }
        self.in_flight.insert(session_id.to_owned());
        let shared = shared.clone();
        let session_id = session_id.to_owned();
        self.resyncs.spawn(async move {
            let applied = refresh(&shared, &session_id).await;
            Resynced {
                session_id,
                applied,
            }
        });
    }

    /// A resync finished: apply its follow-up actions and run a queued one. Returns false when
    /// the core is gone.
    fn resynced(&mut self, done: &Resynced) -> bool {
        let Some(shared) = self.weak.upgrade() else {
            return false;
        };
        self.in_flight.remove(&done.session_id);
        if let Some(applied) = &done.applied {
            self.after_apply(&shared, applied);
        }
        if self.queued.remove(&done.session_id) {
            self.resync(&shared, &done.session_id);
        }
        true
    }
}

/// `session/read` through the gate, merged into the store (routed to the session's channel when it
/// has one). `None` when the read failed (logged; the session stays marked for resync and the next
/// event retries). Counted in `Shared::resync_reads` (the status report shows it: asserts no
/// read happens while sessions merely run).
pub(crate) async fn refresh(shared: &Shared, session_id: &str) -> Option<Applied> {
    shared.resync_reads.fetch_add(1, Ordering::Relaxed);
    let params = json!({"sessionId": session_id});
    let reply = shared.gate.call("session/read", params, None).await;
    let state = match reply {
        Ok(reply) => reply.result.get("state").cloned(),
        Err(error) => {
            warn!(session_id, %error, "resync read failed");
            return None;
        }
    };
    let loaded = state.map(|s| shared.store.load(s, LoadKind::Read));
    match loaded {
        Some(Ok(applied)) => Some(applied),
        Some(Err(error)) => {
            warn!(session_id, %error, "resync result could not be loaded");
            None
        }
        None => {
            warn!(session_id, "resync result has no state");
            None
        }
    }
}

/// How long a delayed re-read waits (the safety net after a read that could not settle a held event).
pub(crate) const DELAYED_RESYNC: Duration = Duration::from_secs(3);

/// Lets the pump's idle tick be at most this coarse.
pub(crate) const MAX_IDLE_CHECK: Duration = Duration::from_secs(30);

#[cfg(test)]
#[path = "ask/delivery_tests.rs"]
mod delivery_tests;
