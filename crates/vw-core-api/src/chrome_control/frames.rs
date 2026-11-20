//! The agent control frame on helper browser tabs (Chrome, Edge). The app tells the helper what each of a chat's
//! sessions shows (`control.frame`: working, paused or off, the chat name and the chat's saved colour index) and applies
//! the person's Pause / Resume clicks the helper reports in its session rows (`personPause`).
//!
//! Depends on: the owner pipe (`chrome_session` `frame`), `work` (the chat's sessions and their
//! state), the paused-tool module. Called by: the gateway around an agent call, the person's pause routes, turn end,
//! Stop and the owner's status poll.
//! Invariants:
//! - The frame is information, never authority: the app's own checks decide every call; a frame update cannot
//!   grant or revive anything.
//! - Only a click the helper attributes to the person (its `personPause` sequence) pauses or resumes; the sequence
//!   is applied once per session and acknowledged back (`ackPause`) so the helper keeps the person's state until the
//!   app has seen it.
//! - An update is sent only when what the frame shows changes (never periodically).
use std::collections::HashMap;
use vw_platform::sync::LockRecover;

use serde_json::{Value, json};

use crate::core::Shared;
use crate::paused_tools::BROWSER;

/// Longest chat name on the frame.
const LABEL_CHARS: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Shown {
    state: &'static str,
    label: String,
    colour: u8,
    ack: u64,
}

/// Per helper session: what the app last sent and the person's last applied pause sequence.
#[derive(Default)]
pub(crate) struct Frames {
    shown: HashMap<String, Shown>,
    confirmed: HashMap<String, Shown>,
    deliveries: HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>,
    seen: HashMap<String, u64>,
    /// Per-chat frame colour index, kept across restarts (`frame_colours`; 0 = the one default colour). Read from
    /// the chats' records folder at first use.
    colours: Option<super::frame_colours::Colours>,
}

/// Confirm the current chat's controls before its page action can enter the helper queue.
/// The ordinary sync still handles newly opened sessions and turn-end changes.
pub(super) async fn before_execute(shared: &Shared, chat: &str, id: &str) -> Result<(), String> {
    // Direct gateway calls outside a running chat turn have no agent frame to show.
    if !crate::paused_tools_commands::turn_running(shared, chat) {
        return Ok(());
    }
    let delivery = shared
        .chrome_control
        .frames
        .lock_or_recover()
        .deliveries
        .entry(id.to_owned())
        .or_default()
        .clone();
    let _delivery = delivery.lock().await;
    let wanted = shared.channels.canonical(chat);
    let sessions = shared.chrome_control.work.lock_or_recover().sessions();
    let state = sessions
        .into_iter()
        .find(|(session, owner, _)| session == id && shared.channels.canonical(owner) == wanted)
        .map(|(_, _, state)| state)
        .ok_or(
            "Not run: this browser session is no longer available. Open a new browser session.",
        )?;
    let state = desired(shared, chat, &state);
    if state == "off" {
        return Ok(());
    }
    let frame_label = label(shared, chat);
    let next = {
        let mut frames = shared.chrome_control.frames.lock_or_recover();
        Shown {
            state,
            label: frame_label,
            colour: frames.colour(colours_dir(shared), chat),
            ack: frames.seen.get(id).copied().unwrap_or(0),
        }
    };
    if shared
        .chrome_control
        .frames
        .lock_or_recover()
        .confirmed
        .get(id)
        == Some(&next)
    {
        return Ok(());
    }
    let answer = shared
        .chrome_control
        .owner
        .call(
            "chrome_session",
            json!({
                "action":"frame", "sessionId":id, "state":next.state, "label":next.label,
                "colour":next.colour, "ackPause":next.ack,
            }),
        )
        .await;
    let result = answer.value();
    if result["isError"] == true
        || result["structuredContent"]["sessionId"] != id
        || result["structuredContent"]["frame"]["state"] != next.state
    {
        return Err("Not run: Vibe Work could not confirm the browser controls. Try the action again after the frame appears.".into());
    }
    let sessions = shared.chrome_control.work.lock_or_recover().sessions();
    let current = sessions
        .into_iter()
        .find(|(session, owner, _)| session == id && shared.channels.canonical(owner) == wanted)
        .map(|(_, _, state)| state);
    if current.as_deref().map(|state| desired(shared, chat, state)) != Some(next.state) {
        return Err("Not run: browser access changed while its controls were being shown. Try the action again.".into());
    }
    let mut frames = shared.chrome_control.frames.lock_or_recover();
    frames.shown.insert(id.to_owned(), next.clone());
    frames.confirmed.insert(id.to_owned(), next);
    Ok(())
}

impl Frames {
    /// The saved colours, read once from `dir` (the app's chat records folder).
    fn colours(&mut self, dir: Option<&std::path::Path>) -> &mut super::frame_colours::Colours {
        self.colours
            .get_or_insert_with(|| super::frame_colours::Colours::load(dir))
    }

    fn colour(&mut self, dir: Option<&std::path::Path>, chat: &str) -> u8 {
        self.colours(dir).get(chat)
    }
}

/// Where the chat colours are kept: with the app's other per-chat records (the remembered chat models).
fn colours_dir(shared: &Shared) -> Option<&std::path::Path> {
    shared.config.sessions_dir.as_deref()
}

/// Every chat's stored frame colour (chats at the default colour are left out), for the built-in browser's frame.
pub(crate) fn colours(shared: &Shared) -> std::collections::BTreeMap<String, u8> {
    shared
        .chrome_control
        .frames
        .lock_or_recover()
        .colours(colours_dir(shared))
        .all()
}

/// The per-chat colour setting: stores `colour` for the chat and shows it on every browser frame of the chat.
///
/// # Errors
/// `invalidParams` for an index outside the palette; `internal` when the setting could not be saved.
pub(crate) fn set_colour(
    shared: &Shared,
    session: &str,
    colour: u8,
) -> Result<(), crate::error::CoreError> {
    use crate::error::{CoreError, ErrorCode};
    if colour >= super::frame_colours::PALETTE {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            format!(
                "The frame colour must be a palette index from 0 to {}.",
                super::frame_colours::PALETTE - 1
            ),
        ));
    }
    let chat = shared.channels.canonical(session);
    let changed = {
        let mut frames = shared.chrome_control.frames.lock_or_recover();
        let colours = frames.colours(colours_dir(shared));
        let changed = colours.set(&chat, colour);
        if changed {
            colours
                .save(colours_dir(shared))
                .map_err(|reason| CoreError::new(ErrorCode::Internal, reason))?;
        }
        changed
    };
    if changed {
        sync_chat(shared, &chat);
    }
    Ok(())
}

fn label(shared: &Shared, chat: &str) -> String {
    crate::title_guard::withhold_text(shared, &shared.names.display_title(&shared.store, chat))
        .chars()
        .filter(|c| !c.is_control())
        .take(LABEL_CHARS)
        .collect()
}

/// What the frame of one session of `chat` shows now.
fn desired(shared: &Shared, chat: &str, session_state: &str) -> &'static str {
    if !matches!(session_state, "active" | "connecting") {
        return "off";
    }
    if !crate::paused_tools_commands::turn_running(shared, chat) {
        return "off";
    }
    if shared
        .paused_tools
        .is_paused(&shared.channels.canonical(chat), BROWSER)
    {
        "paused"
    } else {
        "working"
    }
}

/// Sends `control.frame` for each of the chat's helper sessions whose frame changed.
pub(crate) fn sync_chat(shared: &Shared, chat: &str) {
    let all = shared.chrome_control.work.lock_or_recover().sessions();
    // Canonical ids are compared outside the work lock (no lock nesting with the channel table).
    let wanted = shared.channels.canonical(chat);
    let sessions: Vec<(String, String)> = all
        .into_iter()
        .filter(|(_, owner, _)| shared.channels.canonical(owner) == wanted)
        .map(|(id, _, state)| (id, state))
        .collect();
    if sessions.is_empty() {
        return;
    }
    let label = label(shared, chat);
    let mut updates = Vec::new();
    {
        let mut frames = shared.chrome_control.frames.lock_or_recover();
        let colour = frames.colour(colours_dir(shared), chat);
        for (id, state) in sessions {
            let ack = frames.seen.get(&id).copied().unwrap_or(0);
            let next = Shown {
                state: desired(shared, chat, &state),
                label: label.clone(),
                colour,
                ack,
            };
            // A stopped session never gets a frame again; nothing was shown means nothing to switch off.
            if matches!(state.as_str(), "stopped" | "stopping" | "disconnected") {
                frames.shown.remove(&id);
                frames.confirmed.remove(&id);
                frames.deliveries.remove(&id);
                if state == "stopped" {
                    frames.seen.remove(&id);
                }
                continue;
            }
            let previous = frames.shown.get(&id);
            if previous == Some(&next) || (previous.is_none() && next.state == "off" && ack == 0) {
                continue;
            }
            frames.shown.insert(id.clone(), next.clone());
            frames.confirmed.remove(&id);
            updates.push((id, next));
        }
    }
    for (id, shown) in updates {
        shared.chrome_control.owner.control(json!({
            "action": "frame",
            "sessionId": id,
            "state": shown.state,
            "label": shown.label,
            "colour": shown.colour,
            "ackPause": shown.ack,
        }));
    }
}

/// The owner's status rows: applies each session's new person Pause / Resume once, then updates the frames.
pub(crate) fn observe(shared: &Shared, rows: &[Value]) {
    observe_rows(shared, rows, false);
}

/// One signed person-frame event; strict identity and current work state are checked in the same work lookup.
pub(crate) fn observe_control(shared: &Shared, row: &Value) {
    observe_rows(shared, std::slice::from_ref(row), true);
}

fn observe_rows(shared: &Shared, rows: &[Value], strict: bool) {
    let mut chats = Vec::new();
    for row in rows {
        let (Some(id), Some(seq), Some(state)) = (
            row["sessionId"].as_str(),
            row["personPause"]["seq"].as_u64(),
            row["personPause"]["state"].as_str(),
        ) else {
            continue;
        };
        // An ended session's row never pauses or resumes anything (its forgotten sequence cannot be replayed).
        if matches!(
            row["state"].as_str(),
            Some("stopped" | "stopping" | "disconnected")
        ) {
            continue;
        }
        let (chat, browser) = {
            let work = shared.chrome_control.work.lock_or_recover();
            let Some((_, chat, current_state)) = work
                .sessions()
                .into_iter()
                .find(|(known, _, _)| known == id)
            else {
                continue;
            };
            if matches!(
                current_state.as_str(),
                "stopping" | "stopped" | "disconnected"
            ) {
                continue;
            }
            let Some(profile) = work.profile_of(id) else {
                continue;
            };
            if strict
                && (row["clientSessionId"].as_str() != Some(chat.as_str())
                    || row["profileId"].as_str() != Some(profile)
                    || !matches!(state, "paused" | "resumed")
                    || seq == 0
                    || !matches!(
                        row["state"].as_str(),
                        Some("active" | "connecting" | "idle" | "releasing")
                    ))
            {
                continue;
            }
            let browser = super::agent_browser::Browser::from_profile(profile);
            (chat, Some(browser))
        };
        let fresh = {
            let mut frames = shared.chrome_control.frames.lock_or_recover();
            let seen = frames.seen.entry(id.to_owned()).or_insert(0);
            let fresh = seq > *seen;
            // A higher acknowledgement differs from what was last sent, so the next sync sends it.
            *seen = (*seen).max(seq);
            fresh
        };
        if !fresh {
            continue;
        }
        let canonical = shared.channels.canonical(&chat);
        match state {
            "paused" if crate::paused_tools_commands::turn_running(shared, &canonical) => {
                let place = browser.map_or("the browser", |browser| browser.short_name());
                if shared.paused_tools.pause(&canonical, BROWSER, place) {
                    // The turn may have ended between the check above and the pause.
                    crate::paused_tools_commands::turn_finished(shared, &canonical);
                }
            }
            "resumed" => {
                shared.paused_tools.resume(&canonical, None);
            }
            _ => {}
        }
        if !chats.contains(&chat) {
            chats.push(chat);
        }
    }
    // A rejected or duplicate single notification must not reconcile unrelated frame rows.
    if strict && chats.is_empty() {
        return;
    }
    for chat in chats {
        sync_chat(shared, &chat);
    }
    let known = shared.chrome_control.work.lock_or_recover().sessions();
    let mut frames = shared.chrome_control.frames.lock_or_recover();
    frames
        .shown
        .retain(|id, _| known.iter().any(|(known, _, _)| known == id));
    frames
        .confirmed
        .retain(|id, _| known.iter().any(|(known, _, _)| known == id));
    frames
        .deliveries
        .retain(|id, _| known.iter().any(|(known, _, _)| known == id));
    frames
        .seen
        .retain(|id, _| known.iter().any(|(known, _, _)| known == id));
}

/// Forgets a removed chat's frame state.
pub(crate) fn forget_chat(shared: &Shared, chat: &str) {
    let mut frames = shared.chrome_control.frames.lock_or_recover();
    let colours = frames.colours(colours_dir(shared));
    if colours.set(chat, 0)
        && let Err(reason) = colours.save(colours_dir(shared))
    {
        tracing::warn!(%reason, "a removed chat's frame colour stays saved until the next change");
    }
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
