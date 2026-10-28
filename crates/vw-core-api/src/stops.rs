//! Per-chat Stop generations invalidate earlier sends and tool decisions before the runtime is stopped.
//! Uses channel identity and turn events; dispatch and the permission-answer routes call it.
//! Hook disconnect owns prompt cancellation when a call ends. Explicit Stop closes remaining cards after
//! Vibe has stopped, so a cancelled card cannot release a sibling hook into another model step.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use vw_ask::Outcome;
use vw_gate::{DenyReason, GateError};

use crate::command::Command;
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// Why a checked send was not sent (the refusal's text, also how [`refused_as_stopped`] knows it).
const NOT_SENT: &str = "Not sent: the chat was stopped after this continuation was scheduled.";

/// What the agent reads when an approval from before a Stop (or a turn that ended failed or interrupted) is used.
pub(crate) const APPROVAL_ENDED: &str = "This approval ended when the chat was stopped or its turn ended. Do not run this action unless the person asks again.";

#[derive(Default)]
struct Chat {
    generation: u64,
    stopped_by_person: bool,
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
}

/// The Stop generation of every chat this core has seen stopped.
#[derive(Default)]
pub(crate) struct Stops {
    chats: Mutex<HashMap<String, Chat>>,
}

impl Stops {
    fn chats(&self) -> MutexGuard<'_, HashMap<String, Chat>> {
        self.chats.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A `session/compact` forward of the chat started (`true`) or ended (`false`).
pub(crate) fn note_compaction(shared: &Shared, session: &str, started: bool) {
    let id = shared.channels.canonical(session);
    let mut chats = shared.stops.chats();
    let chat = chats.entry(id).or_default();
    chat.compacting = if started {
        chat.compacting + 1
    } else {
        chat.compacting.saturating_sub(1)
    };
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
        stopped(shared, id);
    }
    sessions
}

/// Invalidate earlier decisions immediately and stop protected-change work.
pub(crate) fn stopped(shared: &Shared, session: &str) {
    let id = shared.channels.canonical(session);
    {
        let mut chats = shared.stops.chats();
        let chat = chats.entry(id.clone()).or_default();
        chat.generation += 1;
        chat.stopped_by_person = true;
    }
    crate::protected_changes::stop(shared, session);
    if id != session {
        crate::protected_changes::stop(shared, &id);
    }
}

/// Settle any remaining prompts after the existing runtime Stop operation has returned.
pub(crate) fn finished(shared: &Shared, sessions: &[String]) {
    for session in sessions {
        shared
            .asks
            .cancel_session(Some(session), "the turn was stopped");
        let id = shared.channels.canonical(session);
        if id != *session {
            shared
                .asks
                .cancel_session(Some(&id), "the turn was stopped");
        }
    }
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
                    }
                }
            }
        }
        // A chat that never saw a Stop is forgotten (its lookups answer 0 either way); a stopped one keeps its
        // generation, so an old continuation never sees 0 again.
        vw_session::Change::Removed { session_id } => {
            let id = shared.channels.canonical(session_id);
            let mut chats = shared.stops.chats();
            if chats.get(&id).is_some_and(|c| c.generation == 0) {
                chats.remove(&id);
            }
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
    Err(CoreError::new(ErrorCode::Denied, NOT_SENT).with_method(method))
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
        reason: DenyReason::Policy(NOT_SENT.to_owned()),
    })
}

/// Whether `error` is a checked send's refusal after a Stop (nothing was sent).
pub(crate) fn refused_as_stopped(error: &CoreError) -> bool {
    error.code == ErrorCode::Denied && error.message.contains(NOT_SENT)
}

/// After a gated send of `method`: a checked `turn/start` (`expected` set) that Vibe accepted started a turn a later
/// Stop must reach; the person's own `turn/start` means the chat now runs their turn.
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
        .filter(|_| method == "turn/start")
    else {
        return;
    };
    let id = shared.channels.canonical(session);
    let mut chats = shared.stops.chats();
    if expected.is_none() {
        for member in crate::stop_tree::family(shared, &id) {
            chats.entry(member).or_default().stopped_by_person = false;
        }
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
