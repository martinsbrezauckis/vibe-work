//! Bounded display state for chat-owned Chrome sessions. It never grants tool authority.
use std::collections::{HashMap, VecDeque};
use vw_platform::sync::LockRecover;

use super::agent_browser::Browser;

use serde_json::Value;
use tokio::time::Instant;

/// The Chrome state shown beside a chat and through the Debug API.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChromeWorkView {
    /// The owning app chat.
    pub session_id: String,
    /// Active, connecting, releasing, idle, stopping, stopped or disconnected.
    pub state: String,
    /// Browser of this chat's selected or last browser session, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub browser: Option<String>,
}

/// Display the bounded work rows with their existing profile binding, without granting browser authority.
pub(super) fn view(chrome: &super::ChromeControl) -> Vec<ChromeWorkView> {
    let mut rows = chrome.work.lock_or_recover().view();
    let profiles = chrome.chat_profiles.lock_or_recover();
    for row in &mut rows {
        row.browser = profiles.get(&row.session_id).map(|profile| {
            if cfg!(feature = "your-safari") && profile.starts_with("safari:") {
                "Safari"
            } else if cfg!(feature = "your-firefox") && profile.starts_with("firefox:") {
                "Firefox"
            } else if cfg!(feature = "your-edge") && profile.starts_with("edge:") {
                "Edge"
            } else {
                "Chrome"
            }
            .into()
        });
    }
    rows
}

struct Session {
    id: String,
    profile: String,
    row: ChromeWorkView,
    opened: Instant,
    turn: Option<String>,
    stop: Stop,
    /// Ended because the chat's access mode was lowered (the agent's refusal and the chat notice say so).
    lowered: bool,
}
/// Stop intent is independent of the helper's last display state. A poll can predate acknowledgement.
#[derive(Default, PartialEq, Eq)]
enum Stop {
    #[default]
    None,
    Requested,
    Ended,
}
#[derive(Default)]
pub(super) struct Work(VecDeque<Session>);
impl Work {
    pub(super) fn releases(&self, chat: &str) -> Vec<super::profile_selection::Release> {
        self.0
            .iter()
            .filter(|session| {
                session.row.session_id == chat
                    && !matches!(session.row.state.as_str(), "stopped" | "disconnected")
            })
            .map(|session| super::profile_selection::Release {
                id: session.id.clone(),
                profile: session.profile.clone(),
            })
            .collect()
    }

    pub(super) fn remove_browser(&mut self, browser: Browser) {
        for session in &mut self.0 {
            if browser.owns_profile(&session.profile) {
                session.stop = Stop::Ended;
                session.row.state = "disconnected".into();
            }
        }
    }
    pub(super) fn open(
        &mut self,
        helper: &str,
        chat: &str,
        state: &str,
        profile: &str,
        turn: Option<&str>,
    ) {
        self.0.retain(|session| session.id != helper);
        for session in &mut self.0 {
            if session.row.session_id == chat
                && Browser::from_profile(&session.profile) == Browser::from_profile(profile)
            {
                session.row.state = "stopped".into();
                session.stop = Stop::Ended;
            }
        }
        self.0.push_back(Session {
            id: helper.into(),
            profile: profile.into(),
            row: ChromeWorkView {
                session_id: chat.into(),
                state: display_state(state).into(),
                browser: None,
            },
            opened: Instant::now(),
            turn: turn.map(str::to_owned),
            stop: Stop::None,
            lowered: false,
        });
        while self.0.len() > 128 {
            self.0.pop_front();
        }
    }
    pub(super) fn release(&mut self, chat: &str) {
        for session in &mut self.0 {
            if session.row.session_id == chat && session.stop == Stop::None {
                session.row.state = "releasing".into();
                session.opened = Instant::now();
            }
        }
    }
    pub(super) fn resumed(&mut self, id: &str, chat: &str, turn: Option<&str>) {
        for session in &mut self.0 {
            if session.id == id && session.row.session_id == chat && session.stop == Stop::None {
                session.row.state = "active".into();
                session.turn = turn.map(str::to_owned);
                session.opened = Instant::now();
            }
        }
    }
    pub(super) fn end(&mut self, chat: &str, deleted: bool) {
        if deleted {
            self.0.retain(|session| session.row.session_id != chat);
        } else {
            for session in &mut self.0 {
                if session.row.session_id == chat {
                    session.stop = Stop::Ended;
                    session.opened = Instant::now();
                    if !matches!(session.row.state.as_str(), "stopped" | "disconnected") {
                        session.row.state = "stopping".into();
                    }
                }
            }
        }
    }
    /// Mark only this chat's requested helper session before sending its stop.
    pub(super) fn ask_stop(&mut self, helper: &str, chat: &str) {
        for session in &mut self.0 {
            if session.id == helper
                && session.row.session_id == chat
                && matches!(
                    session.row.state.as_str(),
                    "active" | "connecting" | "idle" | "releasing"
                )
            {
                session.row.state = "stopping".into();
                session.stop = Stop::Requested;
                session.opened = Instant::now();
            }
        }
    }
    /// Only the stop call's explicit refusal can release its pending intent; a status poll cannot.
    pub(super) fn stop_refused(&mut self, helper: &str, chat: &str) {
        for session in &mut self.0 {
            if session.id == helper
                && session.row.session_id == chat
                && session.stop == Stop::Requested
            {
                session.stop = Stop::None;
                session.opened = Instant::now();
            }
        }
    }
    /// The shown state of `helper` when it is `chat`'s session (a refusal names what happened to it).
    pub(super) fn state_of(&self, helper: &str, chat: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|session| session.id == helper && session.row.session_id == chat)
            .map(|session| session.row.state.as_str())
    }
    /// A helper Stop fences that browser immediately, before acknowledgement.
    /// Remember it on `stopping`: waiting for `stopped` loses the active state on the intervening poll.
    /// A status read that started before the session opened or its stop was asked says nothing about it.
    pub(super) fn stopped_unasked(
        &self,
        rows: &[Value],
        started: Instant,
    ) -> Vec<super::person_stop::Stopped> {
        self.0
            .iter()
            .filter(|session| {
                session.opened <= started
                    && session.stop == Stop::None
                    && matches!(
                        session.row.state.as_str(),
                        "active" | "connecting" | "idle" | "releasing"
                    )
                    && rows.iter().any(|row| {
                        row["sessionId"] == session.id
                            && matches!(row["state"].as_str(), Some("stopping" | "stopped"))
                            && !super::person_stop::helper_fence(row)
                    })
            })
            .map(|session| super::person_stop::Stopped {
                chat: session.row.session_id.clone(),
                browser: Browser::from_profile(&session.profile),
                turn: session.turn.clone(),
                connection_ended: rows.iter().any(|row| {
                    row["sessionId"] == session.id && super::person_stop::connection_ended(row)
                }),
            })
            .collect()
    }
    /// Every helper session as (helper id, owning chat, shown state), for the agent control frame.
    pub(super) fn sessions(&self) -> Vec<(String, String, String)> {
        self.0
            .iter()
            .map(|session| {
                (
                    session.id.clone(),
                    session.row.session_id.clone(),
                    session.row.state.clone(),
                )
            })
            .collect()
    }
    /// The chat whose session `helper` is, when it is still shown.
    /// The chat's access mode was lowered: marks its live sessions as ended for that reason (before they are ended) and
    /// returns their browsers, once each.
    pub(super) fn lower_mode(&mut self, chat: &str) -> Vec<Browser> {
        let mut browsers = Vec::new();
        for session in &mut self.0 {
            if session.row.session_id == chat
                && session.stop == Stop::None
                && !matches!(session.row.state.as_str(), "stopped" | "disconnected")
            {
                session.lowered = true;
                let browser = Browser::from_profile(&session.profile);
                if !browsers.contains(&browser) {
                    browsers.push(browser);
                }
            }
        }
        browsers
    }
    /// Whether the helper session ended because its chat's access mode was lowered.
    pub(super) fn lowered(&self, helper: &str) -> bool {
        self.0
            .iter()
            .any(|session| session.id == helper && session.lowered)
    }
    pub(super) fn chat_of(&self, helper: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|session| session.id == helper)
            .map(|session| session.row.session_id.as_str())
    }
    /// Trusted profile retained with each helper session, including ended and stopped sessions.
    pub(super) fn profile_of(&self, helper: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|session| session.id == helper)
            .map(|session| session.profile.as_str())
    }
    /// A terminal lease from this chat's earlier turn must never become its new open result.
    pub(super) fn stopped_before(&self, helper: &str, chat: &str, turn: Option<&str>) -> bool {
        self.0.iter().any(|session| {
            session.id == helper
                && session.row.session_id == chat
                && session.turn.as_deref() != turn
                && session.stop != Stop::None
        })
    }
    pub(super) fn view(&self) -> Vec<ChromeWorkView> {
        let mut rows: Vec<ChromeWorkView> = Vec::new();
        for session in &self.0 {
            let row = &session.row;
            if let Some(current) = rows.iter_mut().find(|r| r.session_id == row.session_id) {
                // A chat has one display row; either browser's live work keeps it active.
                if matches!(
                    row.state.as_str(),
                    "active" | "connecting" | "releasing" | "stopping"
                ) || !matches!(
                    current.state.as_str(),
                    "active" | "connecting" | "releasing" | "stopping"
                ) {
                    current.clone_from(row);
                }
            } else {
                rows.push(row.clone());
            }
        }
        rows
    }
    pub(super) fn disconnected(&mut self) {
        for session in &mut self.0 {
            if session.row.state != "stopped" {
                session.row.state = "disconnected".into();
            }
        }
    }
    pub(super) fn observe(
        &mut self,
        rows: &[Value],
        tasks: &mut HashMap<String, String>,
        started: Instant,
    ) -> bool {
        let before = self.view();
        for session in &mut self.0 {
            if session.opened > started {
                continue;
            }
            let state = rows
                .iter()
                .find(|row| row["sessionId"] == session.id)
                .and_then(|row| row["state"].as_str());
            if state.is_none() && session.row.state == "stopped" {
                continue;
            }
            if session.row.state == "releasing" && matches!(state, Some("active" | "connecting")) {
                continue;
            }
            if session.stop != Stop::None
                && matches!(state, Some("active" | "connecting" | "idle" | "releasing"))
            {
                continue;
            }
            if matches!(state, Some("stopping" | "stopped")) {
                session.stop = Stop::Ended;
            }
            session.row.state = display_state(state.unwrap_or("disconnected")).into();
        }
        tasks.retain(|id, _| {
            self.0.iter().any(|session| {
                session.id == *id
                    && matches!(
                        session.row.state.as_str(),
                        "active" | "connecting" | "releasing" | "idle" | "stopping"
                    )
            })
        });
        before != self.view()
    }
}
fn display_state(state: &str) -> &str {
    match state {
        "active" | "connecting" | "releasing" | "idle" | "stopping" | "stopped" => state,
        _ => "disconnected",
    }
}

#[cfg(test)]
#[path = "work_tests.rs"]
mod tests;
