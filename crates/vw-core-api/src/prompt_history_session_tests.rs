//! The core keeps temporary chats out of the shared prompt history itself, whichever window or client sends the
//! prompt.
//!
//! PREDICT: before the history command named its chat, `promptHistory` had no session id (the command refuses the
//! unknown `sessionId` field), so the core appended any text it was given and a temporary chat started outside this
//! window (Debug API, or a failed temporary list read at start) left its prompt in the CLI history. The first test
//! fails at parsing the command.
use serde_json::json;

fn history(text: &str, session: Option<&str>) -> crate::Command {
    let mut command = json!({"type":"promptHistory","text":text});
    if let Some(id) = session {
        command["sessionId"] = json!(id);
    }
    serde_json::from_value(command).expect("promptHistory with a session id")
}

#[tokio::test]
async fn a_temporary_chats_prompt_is_never_appended_by_the_core() {
    let f = crate::feature_test_core::Fixture::new().await;
    let path = f.root.join("cli/vibehistory");
    f.core.shared.temporary.add("temp-1").unwrap();
    for command in [
        history("temporary plan", Some("temp-1")),
        // A prompt without its chat cannot be checked, so it is not kept either.
        history("unattributed prompt", None),
    ] {
        f.core.execute(crate::Origin::Ui, command).await.unwrap();
        assert!(!path.exists(), "nothing may be written");
    }
    f.core
        .execute(
            crate::Origin::Ui,
            history("ordinary prompt", Some("chat-1")),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "\"ordinary prompt\"\n"
    );
    f.finish().await;
}
