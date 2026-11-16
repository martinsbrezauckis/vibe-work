//! Person-owned pause of one agent tool per chat (today: the browser). While a tool is paused, the agent's next call
//! of that tool waits instead of running: Resume lets it run, Stop or the end of the turn ends it without running,
//! and a deadline ends it with a plain "not run" answer. The chat shows a written line while the agent waits.
//!
//! Depends on: tokio (`watch`), core events. Called by: the agent tools' pre-tool receipt (the long wait, which has
//! no transport deadline), the browser gateways (a short wait for a pause that started after the receipt), the
//! person's commands (`PausedTools`), the helper's person-pause observer, Stop (`stops::stopped`) and turn end.
//! Invariants:
//! - Only the person pauses or resumes (UI, Debug API acting as the person, or a trusted helper click). The agent has
//!   no tool that reaches this module's `pause`/`resume`.
//! - A held call never runs after Stop or after its turn ended: those outcomes are final for every waiter of that
//!   pause, and a later Resume cannot revive them.
//! - The view lists exactly the paused tools of a chat, with whether a call is waiting; the UI's line follows it.
//! - A held call that Stop or the turn's end ended is recorded by its Vibe session and call id at that moment, before
//!   its answer leaves and before the Stop's interrupt, so the chat can name the outcome however the runtime then
//!   reports the call (the hook's answer delivered, delivered late, or the hook cancelled).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use vw_platform::sync::LockRecover;

use tokio::sync::watch;

use crate::paused_tools_view::{PausedTool, PausedToolsView};

/// The agent tool whose access the browser controls pause.
pub(crate) const BROWSER: &str = "browser";
/// How long a held call waits by default before it ends unrun.
pub(crate) const DEFAULT_WAIT: Duration = Duration::from_secs(10 * 60);
/// A gateway's own wait: inside the agent tool's transport deadline (`vw_agent_tools::wire::CALL_TIMEOUT`, 50 s).
pub(crate) const GATEWAY_WAIT: Duration = Duration::from_secs(40);

/// How long a Stop lets the answer of a call it ended travel to the agent's runtime before the interrupt follows.
/// The answer goes app -> pre-tool hook process -> runtime; an interrupt that arrives first cancels the hook, and the
/// runtime then reports the call as refused by the generic safety hook instead of the paused-browser Stop receipt.
pub(crate) const HELD_ANSWER_LEAD: Duration = Duration::from_millis(250);

/// What ended one held call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    /// The tool was not paused: nothing waited.
    NotPaused,
    /// The person resumed: the call runs after the route's normal checks; its result says the page may have changed.
    Resumed,
    /// The person stopped the turn: the call never runs.
    Stopped,
    /// The turn ended while the call waited: it never runs.
    TurnEnded,
    /// The deadline passed while still paused: the call does not run, the tool stays paused.
    TimedOut,
}

/// Agent texts for the outcomes that do not run the call.
pub(crate) const TIMED_OUT: &str = "Not run: the user is still using the browser (paused). Tell them what you were about to do and wait for their answer.";
/// The note the first browser result after a Resume starts with.
pub(crate) const RESUMED_NOTE: &str = "The user paused the browser and used it, then resumed. The page may have changed: read it again before acting.";
/// The note the first Your Chrome / Edge / Firefox result after a Resume starts with: the helper ends the agent's
/// network rules at the Pause (so none acts on the person's own requests), and they do not come back at Resume.
pub(crate) const CHROME_RESUMED_NOTE: &str = "The user paused the browser and used it, then resumed. The page may have changed: read it again before acting. Network rules you added with network.intercept ended at the pause: add them again if you still need them.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    Resumed,
    Stopped,
    TurnEnded,
}

/// One agent call waiting in the pre-tool receipt: the Vibe session that made it (the hook's `session_id`) and the
/// runtime's id for the call (the hook's `tool_call_id`, also the `toolCallId` of Vibe's hook notices for it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeldCall {
    pub(crate) session: String,
    pub(crate) call: String,
}

/// How a held call ended without running, as the chat names it (the `vwHeldCall` mark on its hook notices).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeldEnd {
    /// The person pressed Stop while the call waited.
    Stopped,
    /// The turn ended while the call waited.
    TurnEnded,
}

impl HeldEnd {
    /// The mark's value in a history entry.
    pub(crate) const fn mark(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::TurnEnded => "turnEnded",
        }
    }
}

struct Pause {
    id: u64,
    since_ms: u64,
    /// Where the person paused (shown in the line), e.g. "Chrome" or "the built-in browser".
    place: String,
    waiting: u32,
    /// The waiting calls whose identity is known (the receipt's wait), by waiter token.
    calls: Vec<(u64, HeldCall)>,
}

#[derive(Default)]
struct State {
    next: u64,
    /// chat -> tool -> pause.
    paused: HashMap<String, HashMap<String, Pause>>,
    /// Final outcomes of recent pauses (bounded), read by waiters after their pause left `paused`.
    ended: Vec<(u64, End)>,
    /// chat -> tools whose next result carries [`RESUMED_NOTE`].
    resumed_note: HashMap<String, Vec<String>>,
    /// chat -> when the answers of calls a Stop just ended have had [`HELD_ANSWER_LEAD`] to leave.
    answering: HashMap<String, tokio::time::Instant>,
    /// Recent held calls that Stop or the turn's end ended (bounded, oldest first).
    held_ends: VecDeque<(HeldCall, HeldEnd)>,
    /// Vibe sessions with a newly recorded held end whose view has not been published again yet.
    fresh: Vec<String>,
}

const ENDED_CAP: usize = 256;

/// The module's state; one per core.
pub(crate) struct PausedTools {
    state: Mutex<State>,
    changed: watch::Sender<u64>,
}

impl Default for PausedTools {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            changed: watch::channel(0).0,
        }
    }
}

use vw_changes::now_ms;

/// Decrements the waiter count when a held call ends, including when its future is dropped (hook killed by Stop).
struct Waiting<'a> {
    tools: &'a PausedTools,
    chat: String,
    tool: String,
    id: u64,
    /// This waiter's token in the pause's known calls, when its call identity is known.
    token: Option<u64>,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let changed = {
            let mut state = self.tools.lock();
            state
                .paused
                .get_mut(&self.chat)
                .and_then(|tools| tools.get_mut(&self.tool))
                .filter(|pause| pause.id == self.id)
                .is_some_and(|pause| {
                    pause.waiting = pause.waiting.saturating_sub(1);
                    if let Some(token) = self.token {
                        pause.calls.retain(|(known, _)| *known != token);
                    }
                    true
                })
        };
        if changed {
            self.tools.bump();
        }
    }
}

impl PausedTools {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock_or_recover()
    }

    fn bump(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// A receiver that changes whenever any pause starts, ends or gains/loses a waiter.
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// The person pauses `tool` in `chat`. Returns false when it was already paused (nothing changes).
    pub(crate) fn pause(&self, chat: &str, tool: &str, place: &str) -> bool {
        let started = {
            let mut state = self.lock();
            state.next += 1;
            let id = state.next;
            let tools = state.paused.entry(chat.to_owned()).or_default();
            if tools.contains_key(tool) {
                false
            } else {
                tools.insert(
                    tool.to_owned(),
                    Pause {
                        id,
                        since_ms: now_ms(),
                        place: place.chars().take(60).collect(),
                        waiting: 0,
                        calls: Vec::new(),
                    },
                );
                true
            }
        };
        if started {
            self.bump();
        }
        started
    }

    fn end(&self, chat: &str, tool: Option<&str>, end: End) -> bool {
        let ended = {
            let mut state = self.lock();
            if end != End::Resumed {
                // A note from a Resume whose browser result never came belongs to the ended turn.
                state.resumed_note.remove(chat);
            }
            let Some(tools) = state.paused.get_mut(chat) else {
                return false;
            };
            let names: Vec<String> = tools
                .keys()
                .filter(|name| tool.is_none_or(|tool| tool == name.as_str()))
                .cloned()
                .collect();
            let mut ended = Vec::new();
            for name in &names {
                if let Some(pause) = tools.remove(name) {
                    ended.push((name.clone(), pause));
                }
            }
            if tools.is_empty() {
                state.paused.remove(chat);
            }
            if end == End::Stopped && ended.iter().any(|(_, pause)| pause.waiting > 0) {
                state.answering.insert(
                    chat.to_owned(),
                    tokio::time::Instant::now() + HELD_ANSWER_LEAD,
                );
            }
            let held = match end {
                End::Stopped => Some(HeldEnd::Stopped),
                End::TurnEnded => Some(HeldEnd::TurnEnded),
                End::Resumed => None,
            };
            for (call, held) in held.into_iter().flat_map(|held| {
                ended
                    .iter()
                    .flat_map(|(_, pause)| pause.calls.iter().map(|(_, call)| call.clone()))
                    .map(move |call| (call, held))
            }) {
                if !state.fresh.contains(&call.session) {
                    state.fresh.push(call.session.clone());
                }
                state.held_ends.push_back((call, held));
            }
            while state.held_ends.len() > ENDED_CAP {
                state.held_ends.pop_front();
            }
            for (name, pause) in &ended {
                state.ended.push((pause.id, end));
                if end == End::Resumed {
                    let notes = state.resumed_note.entry(chat.to_owned()).or_default();
                    if !notes.contains(name) {
                        notes.push(name.clone());
                    }
                }
            }
            let overflow = state.ended.len().saturating_sub(ENDED_CAP);
            state.ended.drain(..overflow);
            !ended.is_empty()
        };
        if ended {
            self.bump();
        }
        ended
    }

    /// After a Stop ended calls that were waiting in `chat`: returns once their answers have had time to reach the
    /// agent's runtime, so the Stop's interrupt never overtakes them. Returns at once when no call was waiting.
    pub(crate) async fn answers_delivered(&self, chat: &str) {
        let until = self.lock().answering.remove(chat);
        if let Some(until) = until {
            tokio::time::sleep_until(until).await;
        }
    }

    /// The person resumes `tool` (every paused tool of the chat when `None`): waiting calls run.
    pub(crate) fn resume(&self, chat: &str, tool: Option<&str>) -> bool {
        self.end(chat, tool, End::Resumed)
    }

    /// The person's Stop: every paused tool of the chat ends, waiting calls never run.
    pub(crate) fn stop_chat(&self, chat: &str) -> bool {
        self.end(chat, None, End::Stopped)
    }

    /// The chat's turn ended: pauses end with it (the next turn starts unpaused), waiting calls never run.
    pub(crate) fn turn_ended(&self, chat: &str) -> bool {
        self.end(chat, None, End::TurnEnded)
    }

    /// True while `tool` is paused in `chat`.
    pub(crate) fn is_paused(&self, chat: &str, tool: &str) -> bool {
        self.lock()
            .paused
            .get(chat)
            .is_some_and(|tools| tools.contains_key(tool))
    }

    /// Takes the "page may have changed" note for the first result of `tool` after a Resume.
    pub(crate) fn take_resumed_note(&self, chat: &str, tool: &str) -> Option<&'static str> {
        let mut state = self.lock();
        let notes = state.resumed_note.get_mut(chat)?;
        let at = notes.iter().position(|name| name == tool)?;
        notes.remove(at);
        if notes.is_empty() {
            state.resumed_note.remove(chat);
        }
        Some(RESUMED_NOTE)
    }

    /// How a held call of Vibe session `session` with runtime call id `call` ended without running, if a Stop or a
    /// turn end recorded it (the most recent record counts).
    pub(crate) fn held_end(&self, session: &str, call: &str) -> Option<HeldEnd> {
        self.lock()
            .held_ends
            .iter()
            .rev()
            .find(|(held, _)| held.session == session && held.call == call)
            .map(|(_, end)| *end)
    }

    /// True when any held end is recorded (lets a history projection skip its walk otherwise).
    pub(crate) fn has_held_ends(&self) -> bool {
        !self.lock().held_ends.is_empty()
    }

    /// Takes the Vibe sessions that gained a held end since the last call.
    pub(crate) fn take_fresh(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().fresh)
    }

    /// Waits while `tool` is paused in `chat`, at most `deadline`.
    #[cfg(test)]
    pub(crate) async fn hold(&self, chat: &str, tool: &str, deadline: Duration) -> Hold {
        self.hold_call(chat, tool, deadline, None).await
    }

    /// Waits while `tool` is paused in `chat`, at most `deadline`. A Stop or turn end that ends a call whose identity
    /// is known (`call`) records it ([`Self::held_end`]).
    pub(crate) async fn hold_call(
        &self,
        chat: &str,
        tool: &str,
        deadline: Duration,
        call: Option<HeldCall>,
    ) -> Hold {
        let mut changes = self.subscribe();
        let (id, token) = {
            let mut state = self.lock();
            state.next += 1;
            let next = state.next;
            let Some(pause) = state
                .paused
                .get_mut(chat)
                .and_then(|tools| tools.get_mut(tool))
            else {
                return Hold::NotPaused;
            };
            pause.waiting += 1;
            let token = call.map(|call| {
                pause.calls.push((next, call));
                next
            });
            (pause.id, token)
        };
        self.bump();
        let _waiting = Waiting {
            tools: self,
            chat: chat.to_owned(),
            tool: tool.to_owned(),
            id,
            token,
        };
        let until = tokio::time::Instant::now() + deadline;
        loop {
            {
                let state = self.lock();
                let still = state
                    .paused
                    .get(chat)
                    .and_then(|tools| tools.get(tool))
                    .is_some_and(|pause| pause.id == id);
                if !still {
                    let end = state
                        .ended
                        .iter()
                        .rev()
                        .find(|(ended, _)| *ended == id)
                        .map(|(_, end)| *end);
                    // An outcome that left the bounded record is treated as the safe one: not run.
                    return match end {
                        Some(End::Resumed) => Hold::Resumed,
                        Some(End::TurnEnded) => Hold::TurnEnded,
                        Some(End::Stopped) | None => Hold::Stopped,
                    };
                }
            }
            tokio::select! {
                next = changes.changed() => {
                    if next.is_err() {
                        return Hold::Stopped;
                    }
                }
                () = tokio::time::sleep_until(until) => return Hold::TimedOut,
            }
        }
    }

    /// The paused tools of `chat` for its line.
    pub(crate) fn view(&self, chat: &str) -> PausedToolsView {
        let state = self.lock();
        let mut paused: Vec<PausedTool> = state
            .paused
            .get(chat)
            .map(|tools| {
                tools
                    .iter()
                    .map(|(tool, pause)| PausedTool {
                        tool: tool.clone(),
                        place: pause.place.clone(),
                        since_ms: pause.since_ms,
                        waiting: pause.waiting > 0,
                    })
                    .collect()
            })
            .unwrap_or_default();
        paused.sort_by(|a, b| a.tool.cmp(&b.tool));
        PausedToolsView {
            session_id: chat.to_owned(),
            paused,
        }
    }

    /// Chats with at least one paused tool.
    pub(crate) fn chats(&self) -> Vec<String> {
        self.lock().paused.keys().cloned().collect()
    }
}

/// Publishes the chat's view whenever this module changes (one task per core).
pub(crate) fn publish_changes(shared: &Arc<crate::core::Shared>) {
    let weak = Arc::downgrade(shared);
    let mut changes = shared.paused_tools.subscribe();
    tokio::spawn(async move {
        // Chats shown last time: a chat whose pause ended gets one empty view.
        let mut shown: Vec<String> = Vec::new();
        while changes.changed().await.is_ok() {
            let Some(shared) = weak.upgrade() else { return };
            let mut now = shared.paused_tools.chats();
            now.sort();
            let mut all: Vec<&String> = shown.iter().chain(now.iter()).collect();
            all.sort();
            all.dedup();
            for chat in all {
                let view = shared.paused_tools.view(chat);
                shared.emit(crate::CoreEvent::PausedTools { view });
            }
            shown = now;
            // A held call that just ended without running: the chat's history is shown again with its mark, also on
            // the hook notices that arrived while it waited (`paused_tools_gate::mark_held_calls`).
            for session in shared.paused_tools.take_fresh() {
                crate::ask::tool_calls::changed(&shared, &session);
            }
        }
    });
}

#[cfg(test)]
#[path = "paused_tools_tests.rs"]
mod tests;
