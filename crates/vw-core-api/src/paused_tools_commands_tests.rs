//! The person's next message resumes a paused browser on every route the window and the Debug API send a message by:
//! the typed `StartTurn`/`SteerTurn` and the generic `Call` the composer uses (`turn/start`, `turn/steer`, a queued
//! message and a queued message sent now). A call that is not a message (an edit of a queued message, an interrupt, a
//! read) and a message to another chat leave the pause alone; the agent's own tool calls never reach this route.
//!
//! PREDICT (written before the generic route counted): the composer's `Call {method:"turn/steer"}` and
//! `Call {method:"turn/start"}` leave the browser paused (only the typed commands were matched), so every
//! `Call` case below fails on `is_paused == false`; the typed and the "not a message" cases already pass.
use crate::paused_tools::BROWSER;
use crate::{Command, Origin};
use serde_json::{Value, json};

async fn paused_core(name: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |_, _| {}).await;
    for id in ["a", "b"] {
        let _ = core
            .shared
            .store
            .load(
                json!({
                    "session":{"id":id,"status":{"type":"running","activeTurnId":"t"}},
                    "history":[],"turns":[{"id":"t","status":"in_progress"}]
                }),
                vw_session::LoadKind::New,
            )
            .unwrap();
    }
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    core
}

fn call(method: &str, params: Value) -> Command {
    Command::Call {
        method: method.into(),
        params,
        confirmation_token: None,
    }
}

/// The composer's own params shapes (`ui/src/state/composerSend.ts`).
fn person_messages() -> Vec<(&'static str, Command)> {
    let text = json!([{"type":"text","text":"use the other link"}]);
    vec![
        (
            "typed StartTurn",
            Command::StartTurn {
                session_id: "a".into(),
                message: vec![],
                client_user_message_id: None,
            },
        ),
        (
            "typed SteerTurn",
            Command::SteerTurn {
                session_id: "a".into(),
                expected_turn_id: "t".into(),
                message: vec![],
                client_user_message_id: None,
            },
        ),
        (
            "composer steer",
            call(
                "turn/steer",
                json!({"sessionId":"a","message":text,"clientUserMessageId":"u1","expectedTurnId":"t"}),
            ),
        ),
        (
            "composer send",
            call(
                "turn/start",
                json!({"sessionId":"a","message":text,"clientUserMessageId":"u2"}),
            ),
        ),
        (
            "composer queue",
            call(
                "session/turn/enqueue",
                json!({"sessionId":"a","entries":[{"role":"user","content":text}]}),
            ),
        ),
        (
            "queued message sent now",
            call(
                "session/turn/queue/steer",
                json!({"sessionId":"a","queueItemId":"q1"}),
            ),
        ),
    ]
}

#[tokio::test]
async fn every_person_message_route_resumes_the_paused_browser() {
    for origin in [Origin::Ui, Origin::DebugApi] {
        for (label, command) in person_messages() {
            let core = paused_core("pause-message-routes").await;
            // The offline core has no Vibe: the send itself fails after the message was counted.
            let _ = core.execute(origin, command).await;
            assert!(
                !core.shared.paused_tools.is_paused("a", BROWSER),
                "{label} from {origin:?} resumes"
            );
            core.shutdown().await;
        }
    }
}

#[tokio::test]
async fn calls_that_are_not_a_message_to_this_chat_keep_the_pause() {
    for (label, command) in [
        (
            "edit of a queued message",
            call(
                "session/turn/queue/replace",
                json!({"sessionId":"a","queueItemId":"q1","entries":[]}),
            ),
        ),
        (
            "removal of a queued message",
            call(
                "session/turn/queue/remove",
                json!({"sessionId":"a","queueItemId":"q1"}),
            ),
        ),
        ("read", call("session/read", json!({"sessionId":"a"}))),
        (
            "message to another chat",
            call(
                "turn/steer",
                json!({"sessionId":"b","message":[],"expectedTurnId":"t"}),
            ),
        ),
        ("message without a chat", call("turn/start", json!({}))),
        ("params not an object", call("turn/start", json!("a"))),
        (
            "typed message to another chat",
            Command::StartTurn {
                session_id: "b".into(),
                message: vec![],
                client_user_message_id: None,
            },
        ),
    ] {
        let core = paused_core("pause-not-a-message").await;
        let _ = core.execute(Origin::Ui, command).await;
        assert!(
            core.shared.paused_tools.is_paused("a", BROWSER),
            "{label} keeps the pause"
        );
        core.shutdown().await;
    }
}
