//! Person Stop from trusted helper session state. Uses the existing app interrupt and warning notice routes.
//! Work binds each stop to its owning turn; a delayed observer or interrupt cannot stop a later turn.
use super::agent_browser::Browser;
use crate::{Command, Core, Origin, core::Shared};
use serde_json::{Value, json};
use std::sync::{Arc, PoisonError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stopped {
    pub chat: String,
    pub browser: Browser,
    pub turn: Option<String>,
}

pub(super) fn refusal(browser: Browser) -> String {
    format!(
        "Not run: you pressed Stop in {}. The turn is stopped; do not call browser tools again in this turn.",
        browser.name()
    )
}

pub(super) fn turn(shared: &Shared, chat: &str) -> Option<String> {
    let view = shared.store.view(chat)?;
    view.state["turns"].as_array()?.last()?["id"]
        .as_str()
        .map(str::to_owned)
}

pub(super) fn active_turn(shared: &Shared, chat: &str) -> Option<String> {
    let view = shared.store.view(chat)?;
    let turn = view.state["turns"].as_array()?.last()?;
    (turn["status"] == "in_progress")
        .then(|| turn["id"].as_str().map(str::to_owned))
        .flatten()
}

pub(super) fn received(shared: &Arc<Shared>, stopped: &[Stopped]) {
    let current: Vec<_> = stopped
        .iter()
        .filter(|stop| turn(shared, &stop.chat) == stop.turn)
        .cloned()
        .collect();
    let claimed = shared.chrome_control.person_stopped(&current);
    for stop in claimed {
        let Some(turn) = &stop.turn else { continue };
        if active_turn(shared, &stop.chat).as_ref() != Some(turn) {
            continue;
        }
        let shared = shared.clone();
        tokio::spawn(async move {
            // No await between this check and execute's Stop invalidation. Vibe also checks expectedTurnId.
            if active_turn(&shared, &stop.chat) != stop.turn {
                return;
            }
            let _ = shared.store.apply(vw_session::Notification::new("warning", json!({
                "sessionId":stop.chat, "turnId":stop.turn,
                "warning":{"code":"browser_stopped", "message":format!("Stopped in {} by you", stop.browser.name())}
            })));
            let core = Core { shared };
            let command = Command::InterruptTurn {
                session_id: stop.chat,
                expected_turn_id: stop.turn.unwrap_or_default(),
            };
            if let Err(error) = core.execute(Origin::Ui, command).await {
                tracing::warn!(method = ?error.method, code = ?error.code, "Browser Stop interrupt failed");
            }
        });
    }
}

/// The turn and stop intent of one helper call, captured before sending and checked before binding its result.
pub(super) struct PendingCall {
    pub turn: Option<String>,
    active: Option<String>,
    generation: u64,
    stop: Option<String>,
    open: bool,
}

impl PendingCall {
    pub(super) fn new(shared: &Shared, chat: &str, tool: &str, args: &Value) -> Self {
        let stop = (tool == "chrome_session"
            && matches!(args["action"].as_str(), Some("stop" | "delete")))
        .then(|| args["sessionId"].as_str().map(str::to_owned))
        .flatten();
        if let Some(id) = &stop {
            shared
                .chrome_control
                .work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ask_stop(id, chat);
        }
        Self {
            turn: turn(shared, chat),
            active: active_turn(shared, chat),
            generation: crate::stops::generation(shared, chat),
            stop,
            open: tool == "chrome_session" && args["action"] == "open",
        }
    }

    pub(super) fn refusal(&self, shared: &Shared, chat: &str, browser: Browser) -> Option<String> {
        shared
            .chrome_control
            .stopped_by_person(chat, browser, self.turn.as_deref())
            .then(|| refusal(browser))
    }

    pub(super) fn finish(
        &self,
        shared: &Shared,
        chat: &str,
        browser: Browser,
        answer: &super::owner::Answer,
    ) -> Result<(), String> {
        if let Some(id) = &self.stop
            && let super::owner::Answer::Helper(value) = answer
            && (value["isError"] == true || value["structuredContent"]["error"].is_object())
        {
            shared
                .chrome_control
                .work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stop_refused(id, chat);
        }
        let stopped = shared
            .chrome_control
            .stopped_by_person(chat, browser, self.turn.as_deref());
        if stopped
            || self.generation != crate::stops::generation(shared, chat)
            || self.active != active_turn(shared, chat)
            || shared.store.view(chat).is_none()
        {
            // A late open belongs to the old turn. Release only its result, never the next turn's sessions.
            if self.open
                && let Some(id) = answer.value()["structuredContent"]["sessionId"].as_str()
            {
                shared
                    .chrome_control
                    .owner
                    .control(json!({"action":"stop","sessionId":id}));
            }
            return Err(if stopped {
                refusal(browser)
            } else {
                "Not run: this chat's Chrome work ended.".into()
            });
        }
        Ok(())
    }
}
