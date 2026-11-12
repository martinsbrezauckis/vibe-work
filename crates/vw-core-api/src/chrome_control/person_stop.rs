//! Person Stop from trusted helper session state. Uses the existing app interrupt and warning notice routes.
//! Work binds each stop to its owning turn; a delayed observer or interrupt cannot stop a later turn.
//! Invariant: the chat notice says "by you" only when the helper gives no connection-loss reason for the stop.
use super::agent_browser::Browser;
use crate::{Command, Core, Origin, core::Shared};
use serde_json::{Value, json};
use std::sync::{Arc, PoisonError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stopped {
    pub chat: String,
    pub browser: Browser,
    pub turn: Option<String>,
    /// The helper ended the session because its browser connection or owned window ended
    /// (`suspensionReason` `ADAPTER_DISCONNECTED` and the like), not on a person's Stop.
    pub connection_ended: bool,
}

/// Helper `suspensionReason` values that end a session without anyone pressing Stop: the browser's adapter
/// disconnected or its owned window closed (`ADAPTER_DISCONNECTED`), the browser session was replaced
/// (`SESSION_CHANGED`), or the lease ran out (`LEASE_EXPIRED`). `STOP_REQUESTED`, an absent reason (older
/// helpers) and unknown values keep the person attribution.
pub(super) fn connection_ended(row: &Value) -> bool {
    matches!(
        row["suspensionReason"].as_str(),
        Some("ADAPTER_DISCONNECTED" | "SESSION_CHANGED" | "LEASE_EXPIRED")
    )
}

/// The chat's warning line for a stop the app did not ask for.
pub(super) fn notice(stop: &Stopped) -> String {
    if stop.connection_ended {
        format!(
            "Stopped in {}: the browser connection ended",
            stop.browser.name()
        )
    } else {
        format!("Stopped in {} by you", stop.browser.name())
    }
}

pub(super) fn refusal(browser: Browser) -> String {
    format!(
        "Not run: the user pressed Stop in {}. Do not use the browser again in this turn; tell them what you were doing and wait for their answer.",
        browser.name()
    )
}

/// A completed turn cannot regain browser authority; every route gives the same next step.
pub(super) fn ended(browser: Browser) -> String {
    browser.words("Not run: this chat's {browser} work ended with this turn. Finish the reply; the browser can be used again in the next turn.")
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
            let _ = shared.store.apply(vw_session::Notification::new(
                "warning",
                json!({
                    "sessionId":stop.chat, "turnId":stop.turn,
                    "warning":{"code":"browser_stopped", "message":notice(&stop)}
                }),
            ));
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
    binding_generation: u64,
    browser_generations: [u64; 4],
    stop: Option<String>,
    open: bool,
    profile: Option<(String, u64)>,
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
            binding_generation: super::profile_bindings::generation(&shared.chrome_control, chat),
            browser_generations: [
                Browser::Chrome,
                Browser::Edge,
                Browser::Safari,
                Browser::Firefox,
            ]
            .map(|browser| shared.chrome_control.browser_generation(browser)),
            stop,
            open: tool == "chrome_session" && args["action"] == "open",
            profile: super::receipt::profile(shared, tool, args).map(|id| {
                let generation = shared
                    .chrome_control
                    .profiles
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .generation(&id);
                (id, generation)
            }),
        }
    }

    pub(super) fn refusal(&self, shared: &Shared, chat: &str, browser: Browser) -> Option<String> {
        if self.binding_generation
            != super::profile_bindings::generation(&shared.chrome_control, chat)
        {
            return Some("This chat changed browser profiles; the old result is unconfirmed. Check the selected profile before continuing.".into());
        }
        let revoked = self.profile.as_ref().filter(|(id, generation)| {
            *generation
                != shared
                    .chrome_control
                    .profiles
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .generation(id)
        });
        if let Some((id, _)) = revoked {
            return Some(format!(
                "{} was switched off; the result is unconfirmed. Check this profile before trying again.",
                super::profiles::label(&shared.chrome_control, id)
            ));
        }
        if self.browser_disabled(shared, browser) {
            return Some(format!(
                "{} work was stopped by its switch. Open a fresh {} session after it is enabled.",
                browser.short_name(),
                browser.short_name()
            ));
        }
        shared
            .chrome_control
            .stopped_by_person(chat, browser, self.turn.as_deref())
            .then(|| refusal(browser))
    }

    fn browser_disabled(&self, shared: &Shared, browser: Browser) -> bool {
        let index = match browser {
            Browser::Chrome => 0,
            Browser::Firefox => 3,
            Browser::Edge => 1,
            Browser::Safari => 2,
        };
        self.browser_generations[index] != shared.chrome_control.browser_generation(browser)
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
            || self.refusal(shared, chat, browser).is_some()
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
            return Err(if let Some(reason) = self.refusal(shared, chat, browser) {
                reason
            } else if stopped {
                refusal(browser)
            } else {
                ended(browser)
            });
        }
        Ok(())
    }
}
