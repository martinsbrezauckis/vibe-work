//! A reconnect wakes only the active turn that received a connection refusal. Uses normal turn/steer and Stop guards.
//! The gateway records waiters; trusted helper status updates deliver them. No action is retried.
use super::{AddonState, agent_browser::Browser};
use crate::{Core, core::Shared};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Default)]
pub(super) struct Waiting(Mutex<HashMap<String, Notice>>);

pub(super) struct Notice {
    session: String,
    turn: String,
    generation: u64,
    browser: Browser,
}

fn turn(shared: &Shared, session: &str) -> Option<String> {
    shared
        .store
        .view(session)?
        .state
        .pointer("/session/status/activeTurnId")?
        .as_str()
        .map(str::to_owned)
}

pub(super) fn refused(shared: &Shared, session: &str, browser: Browser, answer: &Value) {
    if !browser.was_paired(shared)
        || !matches!(
            answer["vwMcpResult"]["structuredContent"]["addon"]["stateCode"].as_str(),
            Some("connecting" | "turnedOffInChrome" | "chromeClosed" | "notConnected")
        )
    {
        return;
    }
    let Some(turn) = turn(shared, session) else {
        return;
    };
    let generation = crate::stops::turn_generation(shared, session, Some(&turn));
    shared
        .chrome_control
        .owner
        .connection_waiting
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            session.into(),
            Notice {
                session: session.into(),
                turn,
                generation,
                browser,
            },
        );
}

pub(super) fn take(shared: &Shared) -> Vec<Notice> {
    let mut waiting = shared
        .chrome_control
        .owner
        .connection_waiting
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut ready = Vec::new();
    let mut remaining = HashMap::new();
    for (session, notice) in waiting.drain() {
        if turn(shared, &session).as_deref() != Some(&notice.turn)
            || crate::stops::generation(shared, &session) != notice.generation
        {
            continue;
        }
        if super::availability::live(shared, notice.browser)
            && notice.browser.addon(shared).state == AddonState::Connected
        {
            ready.push(notice);
        } else {
            remaining.insert(session, notice);
        }
    }
    *waiting = remaining;
    ready
}

pub(super) fn connected(shared: &Arc<Shared>) {
    for notice in take(shared) {
        let shared = shared.clone();
        tokio::spawn(async move {
            let _ = notice.send(shared).await;
        });
    }
}

impl Notice {
    async fn send(self, shared: Arc<Shared>) -> Result<vw_gate::Reply, crate::CoreError> {
        let text = format!(
            "{} is connected again. Read chrome_status for the current connection before continuing.",
            self.browser.name()
        );
        Core { shared }.forward_unless_stopped("turn/steer", json!({"sessionId":self.session,"expectedTurnId":self.turn,"message":[{"type":"text","text":text}]}), self.generation).await
    }
}

#[cfg(test)]
#[cfg(feature = "your-edge")]
#[path = "connection_notice_tests.rs"]
mod tests;
