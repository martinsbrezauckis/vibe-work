//! The built-in browser's input block while the agent controls a chat's browser: the person's pointer and keyboard
//! input in the chat's pages the agent controls (its own web pages and one handed to it; never the person's own pages
//! or the App preview, `PreviewService::set_agent_input_block`) is swallowed (the engine's native watch; rule in
//! `vw_platform::native_click::person::block`) for exactly as long as the agent control frame shows Working. The
//! frame's Pause, Resume and Stop sit in the app outside the page and stay usable; Pause lifts the block at once and
//! the agent's calls wait until Resume (`paused_tools`).
//!
//! Working = a turn of the chat runs, the person has not paused the browser, the Browser strip is not stopped, and
//! the agent has a browser action in flight, finished one within `vw_browser::ACTIVE_FOR_MS`, or used the browser
//! earlier in the same turn and still holds it (a shell step between two browser steps keeps it Working; a question to
//! the person releases it; the frame's state in the Browser tab reads the same rule, `browser::browser_state`). The engine holds the block as a lease of `ACTIVE_FOR` that
//! ends by itself; the watch renews it while the rule holds, so a lost update never keeps the person out for long.
//!
//! Re-decided when the chat's browser activity changes (action start/end, strip Stop/Resume), when a pause starts or
//! ends, when a turn ends, and every `ACTIVE_FOR / 3` for the chats that hold a lease.
//! Depends on: the preview hub (browser state, views), `paused_tools`, the store (turn running). Called by: the preview
//! hub's start (the watch task) and `paused_tools_commands::turn_finished`.
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::core::Shared;
use crate::paused_tools::BROWSER;
use vw_platform::sync::LockRecover;

/// How long the block lasts after the agent's last browser activity (the frame's Working window).
const ACTIVE_FOR: Duration = Duration::from_millis(vw_browser::ACTIVE_FOR_MS);

/// The facts the block depends on, read at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// Independent yes/no facts read at one moment; `lease` combines them in one place.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Facts {
    pub(crate) turn_running: bool,
    /// The person took the browser back: paused it, or stopped the Browser strip.
    pub(crate) person_has_it: bool,
    pub(crate) in_flight: bool,
    /// How long ago the agent's last browser action ended; `None` when it has none.
    pub(crate) since_agent: Option<Duration>,
    /// The agent used this chat's browser during the turn that runs now.
    pub(crate) acted_this_turn: bool,
}

/// Whether the browser use recorded for turn `recorded` belongs to the turn running now (`current`).
pub(crate) fn acted(recorded: Option<&str>, current: Option<&str>) -> bool {
    current.is_some() && recorded == current
}

/// How much longer the block holds from now; `None`: no block.
#[must_use]
pub(crate) fn lease(facts: Facts) -> Option<Duration> {
    if !facts.turn_running || facts.person_has_it {
        return None;
    }
    if facts.in_flight || facts.acted_this_turn {
        // Renewed when the action ends (a change of the chat's browser state) and by the watch's tick while the
        // turn runs, so a lost update still ends it within `ACTIVE_FOR`.
        return Some(ACTIVE_FOR);
    }
    facts
        .since_agent
        .and_then(|since| ACTIVE_FOR.checked_sub(since))
        .filter(|left| !left.is_zero())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The id of `chat`'s turn when it runs now.
fn running_turn(shared: &Shared, chat: &str) -> Option<String> {
    shared.store.view(chat).and_then(|view| {
        view.state["turns"]
            .as_array()
            .and_then(|turns| turns.last())
            .filter(|turn| turn["status"] == "in_progress")
            .and_then(|turn| turn["id"].as_str().map(str::to_owned))
    })
}

/// Whether the agent used `owner`'s browser during the turn that runs now (the frame shows Working for the rest of
/// that turn, as long as the block holds).
pub(crate) fn acted_this_turn(shared: &Shared, owner: &str) -> bool {
    let Some(hub) = shared.preview.as_ref() else {
        return false;
    };
    let chat = shared.channels.canonical(owner);
    let current = running_turn(shared, &chat);
    let recorded = hub.agent_turns.lock_or_recover().get(&chat).cloned();
    acted(recorded.as_deref(), current.as_deref()) && controlled(hub, owner)
}

/// Whether the agent still holds any of `owner`'s pages (a question to the person releases them).
fn controlled(hub: &crate::previews::PreviewHub, owner: &str) -> bool {
    std::iter::once(owner.to_owned())
        .chain(hub.service.web_tabs(owner))
        .any(|page| hub.browser.sessions().controlled(&page))
}

/// Reads the facts of `owner`'s browser (its app preview and every web tab) and sets or lifts the block.
pub(crate) fn sync(shared: &Shared, owner: &str) {
    let Some(hub) = shared.preview.as_ref() else {
        return;
    };
    let chat = shared.channels.canonical(owner);
    let current = running_turn(shared, &chat);
    let mut facts = Facts {
        turn_running: crate::paused_tools_commands::turn_running(shared, &chat),
        person_has_it: shared.paused_tools.is_paused(&chat, BROWSER),
        ..Facts::default()
    };
    let now = now_ms();
    let mut last: Option<u64> = None;
    for page in std::iter::once(owner.to_owned()).chain(hub.service.web_tabs(owner)) {
        let state = hub.browser.sessions().state(&page);
        facts.person_has_it |= state.stopped;
        facts.in_flight |= state.in_flight;
        if state.agent_active {
            last = last.max(state.last_agent_at_ms.or(Some(now)));
        }
    }
    facts.since_agent = last.map(|at| Duration::from_millis(now.saturating_sub(at)));
    {
        let mut turns = hub.agent_turns.lock_or_recover();
        let active = facts.in_flight || facts.since_agent.is_some_and(|since| since < ACTIVE_FOR);
        match current.as_ref() {
            Some(turn) if active => {
                turns.insert(chat.clone(), turn.clone());
            }
            Some(_) => {}
            None => {
                turns.remove(&chat);
            }
        }
        facts.acted_this_turn = acted(turns.get(&chat).map(String::as_str), current.as_deref())
            && controlled(hub, owner);
    }
    let until = lease(facts).map(|left| Instant::now() + left);
    hub.service.set_agent_input_block(owner, until);
}

/// Re-decides every chat that is paused or blocked now (the paused-tool module names no chat in its signal).
fn sync_all(shared: &Shared) {
    if shared.preview.is_none() {
        return;
    }
    let mut owners = shared.paused_tools.chats();
    owners.extend(vw_preview::person_block::owners());
    owners.sort_unstable();
    owners.dedup();
    for owner in owners {
        sync(shared, &owner);
    }
}

/// Starts the watch: browser activity changes and pause changes re-decide the block.
pub(crate) fn start(shared: &Arc<Shared>) {
    let Some(hub) = shared.preview.as_ref() else {
        return;
    };
    let mut browser = hub.browser.sessions().changes();
    let mut paused = shared.paused_tools.subscribe();
    let weak: Weak<Shared> = Arc::downgrade(shared);
    tokio::spawn(async move {
        // Renews the leases of turns in which the agent used the browser (each lease lasts `ACTIVE_FOR`).
        let mut renew = tokio::time::interval(ACTIVE_FOR / 3);
        renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let page = tokio::select! {
                _ = renew.tick() => None,
                changed = browser.recv() => match changed {
                    Ok(page) => Some(page),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                changed = paused.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    None
                }
            };
            let Some(shared) = weak.upgrade() else {
                return;
            };
            match page.and_then(|page| {
                shared
                    .preview
                    .as_ref()
                    .and_then(|hub| hub.service.owner_session_id(&page))
            }) {
                Some(owner) => sync(&shared, &owner),
                None => sync_all(&shared),
            }
        }
    });
}

#[cfg(test)]
#[path = "browser_input_block_tests.rs"]
mod tests;
