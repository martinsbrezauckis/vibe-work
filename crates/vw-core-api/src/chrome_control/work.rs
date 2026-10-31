//! Bounded display state for chat-owned Chrome sessions. It never grants tool authority.
use std::collections::{HashMap, VecDeque};

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
}

#[derive(Default)]
pub(super) struct Work(VecDeque<(String, ChromeWorkView, Instant)>);
impl Work {
    pub(super) fn open(&mut self, helper: &str, chat: &str, state: &str) {
        self.0.retain(|(_, row, _)| row.session_id != chat);
        self.0.push_back((
            helper.into(),
            ChromeWorkView {
                session_id: chat.into(),
                state: display_state(state).into(),
            },
            Instant::now(),
        ));
        while self.0.len() > 128 {
            self.0.pop_front();
        }
    }
    pub(super) fn end(&mut self, chat: &str, deleted: bool) {
        if deleted {
            self.0.retain(|(_, row, _)| row.session_id != chat);
        } else {
            for (_, row, _) in &mut self.0 {
                if row.session_id == chat
                    && !matches!(row.state.as_str(), "stopped" | "disconnected")
                {
                    row.state = "stopping".into();
                }
            }
        }
    }
    /// The shown state of `helper` when it is `chat`'s session (a refusal names what happened to it).
    pub(super) fn state_of(&self, helper: &str, chat: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(id, row, _)| id == helper && row.session_id == chat)
            .map(|(_, row, _)| row.state.as_str())
    }
    /// Chats whose live session (active or connecting, not one Vibe Work is stopping) the helper now reports stopped:
    /// the person stopped it in Chrome. A status read that started before the session opened says nothing about it.
    pub(super) fn stopped_unasked(&self, rows: &[Value], started: Instant) -> Vec<String> {
        self.0
            .iter()
            .filter(|(id, row, opened)| {
                *opened <= started
                    && matches!(row.state.as_str(), "active" | "connecting")
                    && rows
                        .iter()
                        .any(|r| r["sessionId"] == *id && r["state"] == "stopped")
            })
            .map(|(_, row, _)| row.session_id.clone())
            .collect()
    }
    /// The chat whose session `helper` is, when it is still shown.
    pub(super) fn chat_of(&self, helper: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(id, _, _)| id == helper)
            .map(|(_, row, _)| row.session_id.as_str())
    }
    pub(super) fn view(&self) -> Vec<ChromeWorkView> {
        self.0.iter().map(|(_, row, _)| row.clone()).collect()
    }
    pub(super) fn disconnected(&mut self) {
        for (_, row, _) in &mut self.0 {
            if row.state != "stopped" {
                row.state = "disconnected".into();
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
        for (id, row, opened) in &mut self.0 {
            if *opened > started {
                continue;
            }
            let state = rows
                .iter()
                .find(|r| r["sessionId"] == *id)
                .and_then(|r| r["state"].as_str());
            if state.is_none() && row.state == "stopped" {
                continue;
            }
            row.state = display_state(state.unwrap_or("disconnected")).into();
        }
        tasks.retain(|id, _| {
            self.0.iter().any(|(helper, row, _)| {
                helper == id && matches!(row.state.as_str(), "active" | "connecting" | "stopping")
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
    fn a_status_started_before_open_does_not_revoke_the_new_session() {
        let started = Instant::now();
        let mut work = Work::default();
        work.open("new", "chat", "active");
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
        work.open("a1", "a", "active");
        work.open("b1", "b", "active");
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
            work.open(&format!("s{i}"), &format!("c{i}"), "active");
        }
        assert_eq!(work.view().len(), 128);
        work.observe(&[], &mut HashMap::new(), Instant::now());
        assert!(work.view().iter().all(|r| r.state == "disconnected"));
        work.open("replacement", "c999", "page-controlled-text");
        assert_eq!(work.view().len(), 128);
        assert_eq!(work.view().last().unwrap().state, "disconnected");
    }
}
