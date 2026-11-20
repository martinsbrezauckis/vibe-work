//! Per-chat Stop generations invalidate earlier sends and tool decisions before the runtime is stopped.
//! Depends on: core channels, gate errors and session events. Called by: dispatch and permission continuations.
//! Uses channel identity and turn events; dispatch and the permission-answer routes call it.
//! Hook disconnect owns prompt cancellation when a call ends. Explicit Stop closes remaining cards after
//! Vibe has stopped, so a cancelled card cannot release a sibling hook into another model step.
//! A person's Stop ends the turn it stopped, not the chat: once that Stop has finished and Vibe took it, a card or
//! turn start of any other turn (a scheduled loop's run, a queued message, a turn Vibe starts on its own) lifts it.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use vw_platform::sync::LockRecover;

use serde_json::{Value, json};
use vw_ask::Outcome;
use vw_gate::{DenyReason, GateError};

use crate::command::Command;
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// What the agent reads when an approval from before a Stop (or a turn that ended failed or interrupted) is used.
pub(crate) const APPROVAL_ENDED: &str = "This approval ended when the chat was stopped or its turn ended. Do not run this action unless the user asks again.";

#[derive(Default)]
struct Chat {
    generation: u64,
    stopped_by_person: bool,
    /// The generation the person's latest Stop moved the chat to: a message sent at a generation below it was on its
    /// way when that Stop came and does not lift it (`sent`). A later failed or interrupted turn raises `generation`
    /// but not this.
    stop_generation: u64,
    /// The last failed or interrupted turn counted, so a turn announced again counts once.
    ended: Option<String>,
    /// The turn the last checked app send started, with the generation it was checked under, until that turn ends,
    /// the person starts a turn of their own, or a Stop takes it.
    started: Option<(String, u64)>,
    /// The chat's latest turn, the generation it started under, and whether it ended: an app send for that turn
    /// (on-demand tools, secret-run completion) is bound to it, so a Stop during the turn also stops what it asks for
    /// later ([`turn_generation`]).
    turn: Option<(String, u64, bool)>,
    /// `session/compact` forwards in flight, which hold the chat's send lock until Vibe answers
    /// (`dispatch_compaction::stop_lock`: the chat's Stop does not wait for them).
    compacting: usize,
    /// Stop commands counted for this chat ([`stopped`]) that have not finished ([`finished`]).
    stopping: usize,
    /// While the person's Stop holds: the turns it ends (the turn it named or was pointed at, the chat's running
    /// turn, the turn of the chat's latest card). A late card of one of them closes; at most [`STOPPED_TURNS`].
    stopped_turns: Vec<String>,
    /// The chat the person's Stop named, as opposed to the subagents it reached through that chat.
    aimed: bool,
    /// The named Stop finished and Vibe took it (the turn was interrupted, none was running, or the channel closed):
    /// a turn of this chat other than `stopped_turns` started after it, so it lifts the Stop ([`lift`]).
    settled: bool,
    /// The turn of the chat's latest card: a Stop also ends a turn whose events have not reached the pump yet.
    card_turn: Option<String>,
}

/// The most turns one chat's Stop remembers as stopped (a few per Stop; overlapping Stops add theirs).
const STOPPED_TURNS: usize = 16;

/// Adds `turn` to the turns a Stop ends, once, keeping the newest [`STOPPED_TURNS`].
fn note_turn(turns: &mut Vec<String>, turn: Option<&str>) {
    let Some(turn) = turn.filter(|t| !t.is_empty()) else {
        return;
    };
    if turns.iter().any(|t| t == turn) {
        return;
    }
    if turns.len() == STOPPED_TURNS {
        turns.remove(0);
    }
    turns.push(turn.to_owned());
}

/// The turn the store shows the chat running.
fn active_turn(shared: &Shared, id: &str) -> Option<String> {
    shared.store.view(id).and_then(|view| {
        view.state["session"]["status"]["activeTurnId"]
            .as_str()
            .map(str::to_owned)
    })
}

/// Lifts a settled Stop of chat `id` for it and its subagents when `turn` is none of the turns it ended. True when
/// lifted. A subagent stopped on its own keeps its own Stop.
fn lift(shared: &Shared, chats: &mut HashMap<String, Chat>, id: &str, turn: &str) -> bool {
    let Some(chat) = chats.get(id) else {
        return false;
    };
    if !(chat.stopped_by_person && chat.settled) || chat.stopped_turns.iter().any(|t| t == turn) {
        return false;
    }
    for member in crate::stop_tree::family(shared, id) {
        let chat = chats.entry(member.clone()).or_default();
        if member != id && chat.aimed {
            continue;
        }
        chat.stopped_by_person = false;
        chat.aimed = false;
        chat.settled = false;
    }
    true
}

/// The Stop generation of every chat this core has seen stopped.
#[derive(Default)]
pub(crate) struct Stops {
    chats: Mutex<HashMap<String, Chat>>,
    /// Woken whenever a `session/compact` forward of any chat starts, so a waiting Stop re-checks at once
    /// (`dispatch_compaction::stop_lock`).
    compaction_started: tokio::sync::Notify,
    /// Woken whenever a chat's generation rises (a Stop, a failed or interrupted turn), a chat is removed or
    /// [`conditions_changed`] runs, so a job waiting in [`until_ended`] re-checks at once.
    ended: tokio::sync::Notify,
}

impl Stops {
    fn chats(&self) -> MutexGuard<'_, HashMap<String, Chat>> {
        self.chats.lock_or_recover()
    }
}

/// A `session/compact` forward of the chat started (`true`) or ended (`false`).
pub(crate) fn note_compaction(shared: &Shared, session: &str, started: bool) {
    let id = shared.channels.canonical(session);
    {
        let mut chats = shared.stops.chats();
        let chat = chats.entry(id).or_default();
        chat.compacting = if started {
            chat.compacting + 1
        } else {
            chat.compacting.saturating_sub(1)
        };
    }
    if started {
        shared.stops.compaction_started.notify_waiters();
    }
}

/// Resolves at the next compaction start of any chat. Enable it before checking [`compacting`]: a start between that
/// check and the wait still wakes it.
pub(crate) fn compaction_started(shared: &Shared) -> tokio::sync::futures::Notified<'_> {
    shared.stops.compaction_started.notified()
}

/// True while a `session/compact` forward of the chat is in flight.
pub(crate) fn compacting(shared: &Shared, session: &str) -> bool {
    let id = shared.channels.canonical(session);
    shared
        .stops
        .chats()
        .get(&id)
        .is_some_and(|c| c.compacting > 0)
}

/// Wakes every [`until_ended`] wait to check its conditions again. Called where a job's authority changes without a
/// Stop generation or a chat removal: the person's agent access switch (`agent_access::set`), a chat mode change
/// (`chrome_control::mode_changed`) or load (`sessions`), and a session's tool credentials ending (`session_end`).
pub(crate) fn conditions_changed(shared: &Shared) {
    shared.stops.ended.notify_waiters();
}

/// Resolves once `ended()` is true: a running job's wait for its chat to be stopped, removed or to lose what it
/// runs under (video jobs, secret captures). It checks at once on every generation rise, chat removal and
/// [`conditions_changed`] (registered before each check, so an event between the check and the wait still wakes it);
/// it has no timer. Callers check their conditions again after the job, so a late wake never lets a stale result
/// through.
pub(crate) async fn until_ended(shared: &Shared, ended: impl Fn() -> bool) {
    loop {
        let event = shared.stops.ended.notified();
        tokio::pin!(event);
        event.as_mut().enable();
        if ended() {
            return;
        }
        event.await;
    }
}

/// The chat's current generation.
pub(crate) fn generation(shared: &Shared, session: &str) -> u64 {
    let id = shared.channels.canonical(session);
    shared.stops.chats().get(&id).map_or(0, |c| c.generation)
}

/// The top of `Core::execute`: a Stop command (`interruptTurn`, a raw `turn/interrupt` call, `stopSession`) counts
/// as soon as it arrives, before it waits for the chat's send lock or its session lease.
pub(crate) fn requested(shared: &Shared, command: &Command) -> Vec<String> {
    let session = match command {
        Command::InterruptTurn { session_id, .. } | Command::StopSession { session_id } => {
            Some(session_id.as_str())
        }
        Command::Call { method, params, .. } if method == "turn/interrupt" => {
            params["sessionId"].as_str()
        }
        _ => None,
    };
    let sessions = session.map_or_else(Vec::new, |session| {
        crate::stop_tree::family(shared, session)
    });
    for id in &sessions {
        if let Some(requests) = &shared.model_requests {
            requests.stop(&shared.channels.canonical(id));
        }
        stopped(shared, id);
    }
    if let Some(session) = session {
        let expected = match command {
            Command::InterruptTurn {
                expected_turn_id, ..
            } => Some(expected_turn_id.as_str()),
            Command::Call { params, .. } => params["expectedTurnId"].as_str(),
            _ => None,
        };
        aim(shared, session, expected);
    }
    sessions
}

/// The chat a person's Stop names (its subagents are marked by [`stopped`] only), with the turn it names.
pub(crate) fn aim(shared: &Shared, session: &str, turn: Option<&str>) {
    let id = shared.channels.canonical(session);
    let mut chats = shared.stops.chats();
    let chat = chats.entry(id).or_default();
    chat.aimed = true;
    note_turn(&mut chat.stopped_turns, turn);
}

/// A turn a running Stop interrupts (the turn its interrupt names after [`retarget`], the root's active turn when
/// the chat is stopped as a whole, a subagent's turn): its late cards close too.
pub(crate) fn ends_turn(shared: &Shared, session: &str, turn: &str) {
    let id = shared.channels.canonical(session);
    if let Some(chat) = shared.stops.chats().get_mut(&id)
        && chat.stopping > 0
    {
        note_turn(&mut chat.stopped_turns, Some(turn));
    }
}

/// Invalidate earlier decisions immediately and stop protected-change work.
pub(crate) fn stopped(shared: &Shared, session: &str) {
    let id = shared.channels.canonical(session);
    let active = active_turn(shared, &id);
    // Fence late helper replies before browser cleanup can wait on its own locks.
    {
        let mut chats = shared.stops.chats();
        let chat = chats.entry(id.clone()).or_default();
        chat.generation += 1;
        if !chat.stopped_by_person {
            chat.stopped_turns.clear();
        }
        chat.stopped_by_person = true;
        chat.stop_generation = chat.generation;
        chat.stopping += 1;
        chat.aimed = false;
        chat.settled = false;
        let latest = chat.turn.as_ref().map(|(turn, _, _)| turn.clone());
        let started = chat.started.as_ref().map(|(turn, _)| turn.clone());
        let card = chat.card_turn.clone();
        for turn in [active, latest, started, card] {
            note_turn(&mut chat.stopped_turns, turn.as_deref());
        }
    }
    shared.stops.ended.notify_waiters();
    // A paused call waiting for the person never runs after Stop.
    shared.paused_tools.stop_chat(session);
    shared.paused_tools.stop_chat(&id);
    crate::chrome_control::stop_chat(shared, session);
    crate::browser::stop_for_chat(shared, session);
    if id != session {
        crate::chrome_control::stop_chat(shared, &id);
        crate::browser::stop_for_chat(shared, &id);
    }
    crate::protected_changes::stop(shared, session);
    if id != session {
        crate::protected_changes::stop(shared, &id);
    }
}

/// Settle any remaining prompts after the existing runtime Stop operation has returned. `took`: Vibe took the Stop
/// ([`took`]); then a later turn of the named chat lifts it ([`lift`]). A Stop Vibe refused holds until the person's
/// next message.
pub(crate) fn finished(shared: &Shared, sessions: &[String], took: bool) {
    for session in sessions {
        let id = shared.channels.canonical(session);
        let stopped = {
            let mut chats = shared.stops.chats();
            let chat = chats.entry(id.clone()).or_default();
            chat.stopping = chat.stopping.saturating_sub(1);
            // A turn that started after Vibe took the interrupt is a new one (a due loop starts as soon as the chat
            // is idle), so nothing seen since is added to the stopped turns.
            chat.aimed &= took && chat.stopped_by_person;
            chat.settled = chat.aimed && chat.stopping == 0;
            chat.stopped_by_person
        };
        // A new turn already lifted the Stop (`taken`): the cards open now are that turn's.
        if !stopped {
            continue;
        }
        shared
            .asks
            .cancel_session(Some(session), "the turn was stopped");
        if id != *session {
            shared
                .asks
                .cancel_session(Some(&id), "the turn was stopped");
        }
    }
}

/// Whether Vibe took a Stop command (`stopping`: the chats it counted, [`requested`]): it answered, or no turn was
/// running any more and the chat has no subagents (that answer skips stopping them).
pub(crate) fn took<T>(result: &Result<T, CoreError>, stopping: &[String]) -> bool {
    match result {
        Ok(_) => true,
        Err(error) => {
            stopping.len() <= 1
                && error.server_code.as_deref() == Some("conflict")
                && error.message.contains("No active turn")
        }
    }
}

/// Vibe took the interrupt of the chat the person's Stop named and its subagents were stopped: from now on a turn
/// of the chat other than the stopped ones is new, even before the Stop command returns.
pub(crate) fn taken(shared: &Shared, session: &str) {
    let id = shared.channels.canonical(session);
    if let Some(chat) = shared.stops.chats().get_mut(&id)
        && chat.stopped_by_person
        && chat.aimed
        && chat.stopping <= 1
    {
        chat.settled = true;
    }
}

/// A card of `turn` registered for `session` (`stop_tree::cancel_late`, before it decides): a card of a turn other
/// than the ones the chat's settled Stop ended lifts that Stop; the turn is remembered for the chat's next Stop.
pub(crate) fn card(shared: &Shared, session: &str, turn: Option<&str>) {
    let Some(turn) = turn.filter(|t| !t.is_empty()) else {
        return;
    };
    let id = shared.channels.canonical(session);
    let mut chats = shared.stops.chats();
    lift(shared, &mut chats, &id, turn);
    chats.entry(id).or_default().card_turn = Some(turn.to_owned());
}

/// Native callbacks racing a personal Stop must be cancelled, including a newly registered child.
pub(crate) fn personally_stopped(shared: &Shared, session: &str) -> bool {
    shared
        .stops
        .chats()
        .get(&shared.channels.canonical(session))
        .is_some_and(|c| c.stopped_by_person)
}

/// The generation an app send for `turn` is bound to: the one the chat's latest turn started under when `turn` is
/// that turn, else the chat's current generation.
pub(crate) fn turn_generation(shared: &Shared, session: &str, turn: Option<&str>) -> u64 {
    let id = shared.channels.canonical(session);
    let chats = shared.stops.chats();
    let Some(chat) = chats.get(&id) else {
        return 0;
    };
    match &chat.turn {
        Some((latest, started, _)) if Some(latest.as_str()) == turn => *started,
        _ => chat.generation,
    }
}

/// Failed or interrupted turns invalidate old decisions once; hook disconnect closes their prompts.
pub(crate) fn observe(shared: &Shared, change: &vw_session::Change) {
    match change {
        vw_session::Change::TurnUpserted { session_id, turn } => {
            let status = turn["status"].as_str();
            let turn_id = turn["id"].as_str();
            let id = shared.channels.canonical(session_id);
            let mut rose = false;
            {
                let mut chats = shared.stops.chats();
                let ended = matches!(
                    status,
                    Some("completed" | "failed" | "interrupted" | "cancelled")
                );
                let chat = chats.entry(id.clone()).or_default();
                if ended
                    && chat
                        .started
                        .as_ref()
                        .is_some_and(|(started, _)| Some(started.as_str()) == turn_id)
                {
                    chat.started = None;
                }
                track_turn(chat, turn_id, status == Some("in_progress"), ended);
                if matches!(status, Some("failed" | "interrupted")) {
                    let counted = turn_id.is_some() && chat.ended.as_deref() == turn_id;
                    if !counted {
                        chat.generation += 1;
                        chat.ended = turn_id.map(str::to_owned);
                        rose = true;
                    }
                }
                // Any turn of the chat other than the ones its settled Stop ended starts fresh.
                if status == Some("in_progress")
                    && let Some(turn) = turn_id
                {
                    lift(shared, &mut chats, &id, turn);
                }
            }
            if rose {
                shared.stops.ended.notify_waiters();
            }
        }
        // A chat that never saw a Stop is forgotten (its lookups answer 0 either way); a stopped one keeps its
        // generation, so an old continuation never sees 0 again.
        vw_session::Change::Removed { session_id } => {
            let id = shared.channels.canonical(session_id);
            {
                let mut chats = shared.stops.chats();
                if chats.get(&id).is_some_and(|c| c.generation == 0) {
                    chats.remove(&id);
                }
            }
            shared.stops.ended.notify_waiters();
        }
        _ => {}
    }
}

/// The pump's view of the chat's latest turn: a new turn the core did not start itself (a queued message) is taken
/// once the previous one ended; a lagging event of an older turn changes nothing.
fn track_turn(chat: &mut Chat, turn_id: Option<&str>, running: bool, ended: bool) {
    let Some(turn_id) = turn_id else {
        return;
    };
    match &mut chat.turn {
        Some((latest, _, done)) if latest.as_str() == turn_id => *done |= ended,
        Some((_, _, false)) => {}
        _ if running => chat.turn = Some((turn_id.to_owned(), chat.generation, false)),
        _ => {}
    }
}

/// A card for a call checked at generation `checked` was just registered as `ask_id`: a Stop that landed in between
/// cancelled nothing (the ask did not exist yet), so the card closes now. True when it closed.
pub(crate) fn opened_after_stop(
    shared: &Shared,
    session: &str,
    checked: u64,
    ask_id: &str,
) -> bool {
    generation(shared, session) != checked
        && shared.asks.settle(
            ask_id,
            Outcome::Cancelled {
                reason: "the turn was stopped".into(),
            },
        )
}

/// The checked forward's early test: `Ok` when no generation is expected (an ordinary send) or the chat named by
/// `params` (`sessionId`) still has it; otherwise the refusal of a continuation the chat's Stop overtook (nothing is
/// sent).
pub(crate) fn still(
    shared: &Shared,
    expected: Option<u64>,
    method: &str,
    params: &Value,
) -> Result<(), CoreError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if params["sessionId"]
        .as_str()
        .map(|id| generation(shared, id))
        == Some(expected)
    {
        return Ok(());
    }
    Err(CoreError::from_gate(
        method,
        &GateError::Denied {
            method: method.to_owned(),
            reason: DenyReason::Stopped,
        },
    ))
}

/// The checked forward under the chat's send lock: the interrupt is pointed at a turn a checked send started
/// ([`retarget`]), then a continuation the chat's Stop overtook is refused ([`still`]).
pub(crate) fn admit(
    shared: &Shared,
    method: &str,
    params: Value,
    expected: Option<u64>,
) -> Result<Value, CoreError> {
    let params = retarget(shared, method, params);
    if method == "turn/interrupt"
        && let (Some(session), Some(turn)) = (
            params["sessionId"].as_str(),
            params["expectedTurnId"].as_str(),
        )
    {
        ends_turn(shared, session, turn);
    }
    still(shared, expected, method, &params)?;
    Ok(params)
}

/// [`still`] right before a gated send (`turn_start::call`, every attempt), as the gate's refusal.
pub(crate) fn sendable(
    shared: &Shared,
    expected: Option<u64>,
    method: &str,
    params: &Value,
) -> Result<(), GateError> {
    still(shared, expected, method, params).map_err(|_| GateError::Denied {
        method: method.to_owned(),
        reason: DenyReason::Stopped,
    })
}

/// Whether `error` is a checked send's refusal after a Stop (nothing was sent).
pub(crate) fn refused_as_stopped(error: &CoreError) -> bool {
    error.code == ErrorCode::Denied && error.server_code.as_deref() == Some("stopped")
}

/// After a gated send of `method`: a checked `turn/start` (`expected` set) that Vibe accepted started a turn a later
/// Stop must reach; the person's own `turn/start` means the chat now runs their turn.
///
/// The person's own message lifts their Stop also when it is queued (`session/turn/enqueue`: voice chat, and a
/// message typed while voice chat is on), so the cards of the turn it starts are not closed as late cards of the
/// stopped one. It lifts it here, when Vibe accepted it, before that turn can ask anything (a card's request can
/// reach the core before the turn's own events). It lifts only a Stop counted before the message was sent (a Stop
/// pressed while it was on its way is meant for the turn it starts), and only once the chat no longer shows the
/// stopped turn running, so a late card of that turn still closes. Any other turn lifts a Stop once that Stop has
/// settled ([`card`], [`observe`]). A subagent's own Stop stays with that subagent: the parent's message does not
/// lift it.
pub(crate) fn sent(
    shared: &Shared,
    expected: Option<u64>,
    at_send: u64,
    method: &str,
    params: &Value,
    result: &Value,
) {
    let Some(session) = params["sessionId"]
        .as_str()
        .filter(|_| matches!(method, "turn/start" | "session/turn/enqueue"))
    else {
        return;
    };
    let id = shared.channels.canonical(session);
    // Read before the chats lock (the order `take_started` uses). Vibe refuses a `turn/start` while a turn runs.
    let running = method != "turn/start"
        && shared.store.view(&id).is_some_and(|view| {
            matches!(
                view.state["session"]["status"]["type"].as_str(),
                Some("running" | "blocked")
            )
        });
    let mut chats = shared.stops.chats();
    let stopped_turn_ended = !running
        || chats
            .get(&id)
            .and_then(|chat| chat.turn.as_ref())
            .is_some_and(|(_, _, done)| *done);
    let stopped_before_send = chats
        .get(&id)
        .is_none_or(|chat| chat.stop_generation <= at_send);
    if expected.is_none() && stopped_turn_ended && stopped_before_send {
        for member in crate::stop_tree::family(shared, &id) {
            let chat = chats.entry(member.clone()).or_default();
            if member != id && chat.aimed {
                continue;
            }
            chat.stopped_by_person = false;
            chat.aimed = false;
            chat.settled = false;
            chat.stopping = 0;
        }
    }
    if method != "turn/start" {
        return;
    }
    let turn = result.pointer("/turn/id").and_then(Value::as_str);
    if let Some(turn) = turn {
        // The turn started under the generation its send was checked or sent in (`at_send`).
        chats.entry(id.clone()).or_default().turn = Some((turn.to_owned(), at_send, false));
    }
    match (expected, turn) {
        (Some(generation), Some(turn)) => {
            chats.entry(id).or_default().started = Some((turn.to_owned(), generation));
        }
        (None, _) => {
            if let Some(chat) = chats.get_mut(&id) {
                chat.started = None;
            }
        }
        (Some(_), None) => {}
    }
}

/// The turn a checked app send started, when a Stop came after that send's check; taken (one Stop reaches it).
/// A record whose turn the store already shows ended, or followed by a later turn, is dropped: its end can reach
/// the pump before the start's caller records it.
fn take_started(shared: &Shared, session: &str) -> Option<String> {
    let id = shared.channels.canonical(session);
    let turns = shared.store.view(&id).map(|v| v.state["turns"].clone());
    let mut chats = shared.stops.chats();
    let chat = chats.get_mut(&id)?;
    let (turn, checked) = chat.started.clone()?;
    let list = turns.as_ref().and_then(Value::as_array);
    let position = list.and_then(|t| t.iter().position(|x| x["id"] == turn.as_str()));
    let current = match (list, position) {
        (Some(t), Some(i)) => i + 1 == t.len() && t[i]["status"] == "in_progress",
        _ => true,
    };
    if !current || checked < chat.generation {
        chat.started = None;
    }
    (current && checked < chat.generation).then_some(turn)
}

/// `turn/interrupt` under the chat's send lock: when a checked app send started a new turn and this Stop came after
/// its check, the interrupt names that turn (the window may still name the turn that had just ended, which Vibe
/// would refuse as stale while the new turn runs on).
pub(crate) fn retarget(shared: &Shared, method: &str, mut params: Value) -> Value {
    if method != "turn/interrupt" {
        return params;
    }
    if let Some(turn) = params["sessionId"]
        .as_str()
        .and_then(|session| take_started(shared, session))
    {
        params["expectedTurnId"] = json!(turn);
    }
    params
}

#[cfg(test)]
#[path = "stops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "stops_wait_tests.rs"]
mod wait_tests;

#[cfg(test)]
#[path = "stops_turn_tests.rs"]
mod turn_tests;
