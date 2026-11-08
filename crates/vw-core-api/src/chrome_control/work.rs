//! Bounded display state for chat-owned Chrome sessions. It never grants tool authority.
use std::collections::{HashMap, VecDeque};

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
    /// Active, connecting, stopping, stopped or disconnected.
    pub state: String,
    /// Browser of this chat's selected or last browser session, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub browser: Option<String>,
}

/// Display the bounded work rows with their existing profile binding, without granting browser authority.
pub(super) fn view(chrome: &super::ChromeControl) -> Vec<ChromeWorkView> {
    let mut rows = chrome
        .work
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .view();
    let profiles = chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        });
        while self.0.len() > 128 {
            self.0.pop_front();
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
                && matches!(session.row.state.as_str(), "active" | "connecting")
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
                    && matches!(session.row.state.as_str(), "active" | "connecting")
                    && rows.iter().any(|row| {
                        row["sessionId"] == session.id
                            && matches!(row["state"].as_str(), Some("stopping" | "stopped"))
                    })
            })
            .map(|session| super::person_stop::Stopped {
                chat: session.row.session_id.clone(),
                browser: Browser::from_profile(&session.profile),
                turn: session.turn.clone(),
            })
            .collect()
    }
    /// The chat whose session `helper` is, when it is still shown.
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
                if matches!(row.state.as_str(), "active" | "connecting" | "stopping")
                    || !matches!(current.state.as_str(), "active" | "connecting" | "stopping")
                {
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
            if session.stop != Stop::None && matches!(state, Some("active" | "connecting")) {
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
                        "active" | "connecting" | "stopping"
                    )
            })
        });
        before != self.view()
    }
}
fn display_state(state: &str) -> &str {
    match state {
        "active" | "connecting" | "stopping" | "stopped" => state,
        _ => "disconnected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_asked_stop_marks_only_the_owned_helper_session() {
        let mut work = Work::default();
        // Two sessions of one chat (Chrome and Edge) guard against using the chat-wide end path. Without Edge a
        // chat holds one live session per browser, so the second session exists only in the Edge build.
        let edge = cfg!(feature = "your-edge");
        work.open("a1", "a", "active", "chrome:p", None);
        if edge {
            work.open("a2", "a", "connecting", "edge:p", None);
        }
        work.open("b1", "b", "active", "chrome:p", None);
        work.ask_stop("a1", "b");
        work.ask_stop("unknown", "a");
        assert_eq!(work.state_of("a1", "a"), Some("active"));
        work.ask_stop("a1", "a");
        assert_eq!(work.state_of("a1", "a"), Some("stopping"));
        assert_eq!(work.state_of("a2", "a"), edge.then_some("connecting"));
        assert_eq!(work.state_of("b1", "b"), Some("active"));
        assert_eq!(
            work.stopped_unasked(
                &[json!({"sessionId":"a1","state":"stopping"})],
                Instant::now(),
            ),
            Vec::<super::super::person_stop::Stopped>::new()
        );
        assert_eq!(
            work.stopped_unasked(
                &[
                    json!({"sessionId":"a1","state":"stopped"}),
                    json!({"sessionId":"a2","state":"stopping"}),
                    json!({"sessionId":"b1","state":"stopped"}),
                ],
                Instant::now(),
            ),
            [
                edge.then(|| super::super::person_stop::Stopped {
                    chat: "a".into(),
                    browser: Browser::from_profile("edge:p"),
                    turn: None
                }),
                Some(super::super::person_stop::Stopped {
                    chat: "b".into(),
                    browser: Browser::from_profile("chrome:p"),
                    turn: None
                }),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn polls_cannot_undo_an_asked_stop_but_an_explicit_refusal_can() {
        for state in ["active", "connecting"] {
            let mut work = Work::default();
            work.open("task", "chat", state, "chrome:p", None);
            let mut tasks = HashMap::from([("task".into(), "chat".into())]);
            let rows = [json!({"sessionId":"task","state":state})];
            let started = Instant::now();
            work.ask_stop("task", "chat");
            assert!(!work.observe(&rows, &mut tasks, started));
            assert_eq!(work.state_of("task", "chat"), Some("stopping"));
            assert!(!work.observe(&rows, &mut tasks, Instant::now()));
            work.stop_refused("task", "chat");
            assert!(work.observe(&rows, &mut tasks, Instant::now()));
            assert_eq!(work.state_of("task", "chat"), Some(state));
            assert_eq!(tasks.get("task").map(String::as_str), Some("chat"));
            // A later unasked stopping poll still identifies the person's Stop immediately.
            assert_eq!(
                work.stopped_unasked(
                    &[json!({"sessionId":"task","state":"stopping"})],
                    Instant::now(),
                ),
                vec![super::super::person_stop::Stopped {
                    chat: "chat".into(),
                    browser: Browser::from_profile("chrome:p"),
                    turn: None
                }]
            );
        }
    }

    #[test]
    fn a_status_started_before_open_does_not_revoke_the_new_session() {
        let started = Instant::now();
        let mut work = Work::default();
        work.open("new", "chat", "active", "p", None);
        let mut tasks = HashMap::from([("new".into(), "chat".into())]);
        work.observe(&[], &mut tasks, started);
        assert_eq!(tasks.len(), 1);
        assert_eq!(work.view()[0].state, "active");
        work.observe(
            &[json!({"sessionId":"new","state":"active"})],
            &mut tasks,
            Instant::now(),
        );
        assert_eq!(tasks.len(), 1);
    }
    #[test]
    fn stop_releases_authority_only_for_that_chat_and_stays_visible() {
        let mut work = Work::default();
        work.open("a1", "a", "active", "p", None);
        work.open("b1", "b", "active", "p", None);
        let mut tasks = HashMap::from([("a1".into(), "a".into()), ("b1".into(), "b".into())]);
        work.end("a", false);
        assert_eq!(work.view()[0].state, "stopping");
        assert!(work.observe(
            &[
                json!({"sessionId":"a1","state":"stopped"}),
                json!({"sessionId":"b1","state":"active"})
            ],
            &mut tasks,
            Instant::now(),
        ));
        assert_eq!(tasks.len(), 1);
        assert!(tasks.contains_key("b1"));
        assert_eq!(work.view()[0].state, "stopped");
        work.disconnected();
        assert_eq!(work.view()[0].state, "stopped");
        assert_eq!(work.view()[1].state, "disconnected");
        work.end("a", true);
        assert_eq!(work.view().len(), 1);
    }
    #[test]
    fn absent_or_unknown_state_cannot_claim_stopped_and_history_is_bounded() {
        let mut work = Work::default();
        for i in 0..1000 {
            work.open(&format!("s{i}"), &format!("c{i}"), "active", "p", None);
        }
        assert_eq!(work.view().len(), 128);
        work.observe(&[], &mut HashMap::new(), Instant::now());
        assert!(work.view().iter().all(|r| r.state == "disconnected"));
        work.open("replacement", "c999", "page-controlled-text", "p", None);
        assert_eq!(work.view().len(), 127);
        assert_eq!(work.view().last().unwrap().state, "disconnected");
    }
}
