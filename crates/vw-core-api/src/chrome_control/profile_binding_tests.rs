//! Stable per-chat profile regressions through the checked gateway and isolated owned-helper fixtures.
use super::{edge_agent_tests::fixture, gateway, test_support};
use serde_json::{Value, json};

#[allow(
    clippy::unwrap_used,
    reason = "test setup requires a saved person default"
)]
async fn connected(name: &str) -> crate::Core {
    let core = fixture(
        name,
        json!([
            {"profileId":"chrome:work","name":"Work","connected":true},
            {"profileId":"chrome:personal","name":"Personal","connected":true},
            {"profileId":"edge:work","name":"Work","connected":true}
        ]),
    )
    .await;
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::SetDefaultProfile {
            profile_id: "chrome:work".into(),
        },
    )
    .await
    .unwrap();
    core
}
async fn open(core: &crate::Core, profile: &str) -> Result<Value, String> {
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
}

#[tokio::test]
async fn agent_cannot_replace_the_chats_profile_binding() {
    let core = connected("profile-binding-fixed").await;
    open(&core, "chrome:work").await.unwrap();
    for profile in ["chrome:personal", "edge:work"] {
        let error = open(&core, profile).await.unwrap_err();
        assert!(
            error.contains("Select") && error.contains("profile"),
            "{error}"
        );
    }
    assert_eq!(
        core.shared.chrome_control.chat_profiles.lock().unwrap()["s"],
        "chrome:work"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn helper_restart_keeps_the_binding_and_persists_it_for_app_restart() {
    let core = connected("profile-binding-restart").await;
    open(&core, "chrome:work").await.unwrap();
    core.shared.chrome_control.reset();
    assert_eq!(
        core.shared
            .chrome_control
            .chat_profiles
            .lock()
            .unwrap()
            .get("s")
            .map(String::as_str),
        Some("chrome:work")
    );
    let restored = super::ChromeControl::new(
        std::path::Path::new("."),
        core.shared.chrome_control.config.clone(),
    );
    assert_eq!(
        restored
            .chat_profiles
            .lock()
            .unwrap()
            .get("s")
            .map(String::as_str),
        Some("chrome:work")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_helper_session_id_cannot_alias_a_different_profiles_receipt() {
    let core = connected("profile-session-alias").await;
    core.shared.chrome_control.work.lock().unwrap().open(
        "same-task",
        "old-chat",
        "stopped",
        "edge:work",
        None,
    );
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_session" {
            json!({"structuredContent":{"sessionId":"same-task","profileId":args["profileId"],"state":"active"}})
        } else {
            json!({"structuredContent":{}})
        }
    });
    let error = open(&core, "chrome:work").await.unwrap_err();
    assert!(
        error.contains("session") && error.contains("profile"),
        "{error}"
    );
    assert_eq!(
        core.shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .profile_of("same-task"),
        Some("edge:work")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_changed_owner_cannot_inherit_the_saved_chat_binding() {
    let core = connected("profile-binding-owner").await;
    test_support::observe_status(&core.shared, &json!({"ownerPublicKey":"first-owner"}));
    open(&core, "chrome:work").await.unwrap();
    test_support::observe_status(&core.shared, &json!({"ownerPublicKey":"changed-owner"}));
    let error = open(&core, "chrome:work").await.unwrap_err();
    assert!(
        error.contains("another Vibe Work identity") && error.contains("Reconnect"),
        "{error}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_wrong_profile_in_an_open_reply_never_rebinds_the_chat() {
    let core = connected("profile-binding-wrong-reply").await;
    let calls = test_support::answering(&core.shared, |tool, _| {
        if tool == "chrome_session" {
            json!({"structuredContent":{"sessionId":"wrong-task","profileId":"edge:work","state":"active"}})
        } else {
            json!({"structuredContent":{}})
        }
    });
    assert!(open(&core, "chrome:work").await.is_err());
    assert!(
        !core
            .shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .contains_key("wrong-task")
    );
    assert_eq!(
        core.shared.chrome_control.chat_profiles.lock().unwrap()["s"],
        "chrome:work"
    );
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_session")
            .count(),
        1
    );
    core.shutdown().await;
}
