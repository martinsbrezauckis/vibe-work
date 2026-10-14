//! Approval callback ownership through the core and scripted fake Vibe.
//! PREDICT: only the callback's session can answer it; user approval and denial each pass once,
//! and the emitted answer keeps the origin that actually sent it.

mod common;

use common::core::{ScriptRig, script_core};
use serde_json::json;
use vw_core_api::{Command, CommandResult, CoreEvent, ErrorCode, Origin};

async fn session(rig: &ScriptRig) -> String {
    match rig
        .core
        .execute(Origin::Ui, Command::StartSession { agent_config: None })
        .await
        .unwrap()
    {
        CommandResult::Session { snapshot, .. } => snapshot.session_id,
        other => panic!("{other:?}"),
    }
}

async fn callback(rig: &ScriptRig, session_id: &str) -> String {
    rig.core
        .execute(
            Origin::Ui,
            Command::StartTurn {
                session_id: session_id.to_owned(),
                message: vec![json!({"type": "text", "text": "go"})],
                client_user_message_id: None,
            },
        )
        .await
        .unwrap();
    for _ in 0..250 {
        let CommandResult::Callbacks { open } = rig
            .core
            .execute(Origin::Ui, Command::ListCallbacks {})
            .await
            .unwrap()
        else {
            panic!("expected callback list");
        };
        if let Some(found) = open.iter().find(|item| item.session_id == session_id) {
            return found.callback_id.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("approval callback did not open");
}

fn answer(session_id: &str, callback_id: &str, decision: &str) -> Command {
    Command::AnswerCallback {
        session_id: session_id.to_owned(),
        callback_id: callback_id.to_owned(),
        output: json!({"type": "approval", "decision": {"type": decision}}),
    }
}

fn answered_with_origin(rig: &mut vw_core_api::CoreSubscription, session_id: &str, origin: Origin) {
    let found = std::iter::from_fn(|| rig.try_next()).any(|event| {
        matches!(
            event,
            CoreEvent::CallbackAnswered { session_id: id, origin: actual, .. }
                if id == session_id && actual == origin
        )
    });
    assert!(found, "callback answer did not retain its origin");
}

#[tokio::test]
async fn approval_card_answers_keep_session_and_origin() {
    let rig = script_core("approval-provenance", &["--script"]).await;
    let first = session(&rig).await;
    let second = session(&rig).await;
    let mut events = rig.core.subscribe(None);

    let denied_id = callback(&rig, &first).await;
    let foreign = rig
        .core
        .execute(Origin::Ui, answer(&second, &denied_id, "approve"))
        .await
        .unwrap_err();
    assert_eq!(foreign.code, ErrorCode::CallbackRefused);
    rig.core
        .execute(Origin::Ui, answer(&first, &denied_id, "deny"))
        .await
        .unwrap();
    answered_with_origin(&mut events, &first, Origin::Ui);
    let repeated = rig
        .core
        .execute(Origin::Ui, answer(&first, &denied_id, "approve"))
        .await
        .unwrap_err();
    // A completed turn can clear the card before this duplicate arrives. Both states refuse it.
    assert!(matches!(
        repeated.code,
        ErrorCode::CallbackRefused | ErrorCode::NotFound
    ));
    assert_eq!(
        rig.received()
            .iter()
            .filter(|method| method.as_str() == "callback/result")
            .count(),
        1,
        "foreign and duplicate answers never reach Vibe"
    );

    let approved_id = callback(&rig, &second).await;
    rig.core
        .execute(Origin::Ui, answer(&second, &approved_id, "approve"))
        .await
        .unwrap();
    answered_with_origin(&mut events, &second, Origin::Ui);

    let third = session(&rig).await;
    let debug_id = callback(&rig, &third).await;
    rig.core
        .execute(Origin::DebugApi, answer(&third, &debug_id, "approve"))
        .await
        .unwrap();
    answered_with_origin(&mut events, &third, Origin::DebugApi);
    rig.core.shutdown().await;
}
