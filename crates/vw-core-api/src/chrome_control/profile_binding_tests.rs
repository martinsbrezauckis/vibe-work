//! Stable per-chat profile regressions through the checked gateway and isolated owned-helper fixtures.
use super::{edge_agent_tests::fixture, gateway, test_support};
use serde_json::{Value, json};

#[allow(
    clippy::unwrap_used,
    reason = "test setup requires a saved person default"
)]
pub(super) async fn connected(name: &str) -> crate::Core {
    let core = fixture(
        name,
        json!([
            {"profileId":"chrome:work","name":"Work","connected":true},
            {"profileId":"chrome:personal","name":"Personal","connected":true},
            {"profileId":"edge:work","name":"Work","connected":true},
            {"profileId":"firefox:agent","name":"Work","connected":true}
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
pub(super) async fn open(core: &crate::Core, profile: &str) -> Result<Value, String> {
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
}

#[path = "profile_binding_storage_tests.rs"]
mod storage_tests;

/// PREDICT: a positive closed-window fence releases only its chat's binding and
/// emits exactly one factual notice while the chat's turn continues.
#[tokio::test]
async fn a_positive_window_close_releases_and_notices_only_its_chat_once() {
    let core = connected("profile-rebind-window-notice").await;
    let _ = core.shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[{"id":"current","status":"in_progress"}]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    test_support::chat(&core.shared, "other");
    super::profile_bindings::remember(
        &core.shared.chrome_control,
        "other",
        "chrome:personal",
        None,
    )
    .unwrap();
    open(&core, "chrome:work").await.unwrap();
    let mut events = core.subscribe(None);
    let closed = json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{
        "sessionId":"task","clientSessionId":"s","profileId":"chrome:work","state":"disconnected",
        "capture":"unknown","suspensionReason":"WINDOW_CLOSED","target":null,"windowId":7,"scope":"window"
    }]}});
    for _ in 0..2 {
        super::owner::update_work_for_test(&core.shared, &closed);
    }
    let mut notices = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_millis(300), events.next()).await
    {
        let value = serde_json::to_value(event).unwrap();
        if value["change"]["params"]["warning"]["code"] == "browser_window_closed" {
            notices.push(value);
        }
    }
    assert_eq!(notices.len(), 1, "one scoped notice per ended connection");
    assert_eq!(notices[0]["change"]["sessionId"], "s");
    assert_eq!(
        notices[0]["change"]["params"]["warning"]["message"],
        "Your Chrome control ended because its window was closed."
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "other").as_deref(),
        Some("chrome:personal")
    );
    assert_eq!(crate::stops::generation(&core.shared, "s"), 0);
    assert_eq!(
        core.shared.store.view("s").unwrap().state["turns"][0]["status"],
        "in_progress"
    );
    core.shutdown().await;
}

/// PREDICT: a live cross-profile refusal names the selected profile and explains
/// close/release or the built-in browser; status carries the same recovery path.
#[tokio::test]
async fn agent_cannot_replace_the_chats_profile_binding() {
    let core = connected("profile-binding-fixed").await;
    open(&core, "chrome:work").await.unwrap();
    for profile in ["chrome:personal", "edge:work", "edge"] {
        let error = open(&core, profile).await.unwrap_err();
        assert!(
            error.contains("Not run:")
                && error.contains("Chrome")
                && error.contains("Close that browser session first")
                && error.contains("built-in browser in parallel")
                && !error.contains("new chat"),
            "{error}"
        );
    }
    let status = super::gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    let selection = status["vwMcpResult"]["structuredContent"]["profileSelection"]
        .as_str()
        .unwrap_or_default();
    assert!(
        selection.contains("Close that browser session")
            && selection.contains("built-in browser is available in parallel"),
        "{selection}"
    );
    assert_eq!(
        core.shared.chrome_control.chat_profiles.lock().unwrap()["s"],
        "chrome:work"
    );
    core.shutdown().await;
}

/// PREDICT: a person's selection without a browser lease is shown as unopened;
/// the person can replace it before any external profile is used.
#[tokio::test]
async fn a_person_can_change_a_selection_before_first_external_use() {
    let core = connected("profile-binding-picked-unopened").await;
    super::profile_selection::select_picked(&core.shared, "s", "chrome:work")
        .await
        .unwrap();
    let view = super::profile_selection::view(&core.shared.chrome_control).unwrap();
    assert_eq!(
        view.iter().find(|row| row.session_id == "s").unwrap().state,
        "unopened"
    );
    super::profile_selection::select(&core.shared, "s", "edge:work")
        .await
        .unwrap();
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("edge:work")
    );
    let view = super::profile_selection::view(&core.shared.chrome_control).unwrap();
    assert_eq!(
        view.iter().find(|row| row.session_id == "s").unwrap().state,
        "unopened"
    );
    open(&core, "edge:work").await.unwrap();
    let view = super::profile_selection::view(&core.shared.chrome_control).unwrap();
    assert_eq!(
        view.iter().find(|row| row.session_id == "s").unwrap().state,
        "selected"
    );
    core.shutdown().await;
}

/// PREDICT: an agent's first successful use of another profile binds that actual
/// profile and emits one chat-scoped notice naming the pick and the used profile.
#[tokio::test]
async fn first_external_use_can_override_a_pick_and_notifies_only_that_chat() {
    let core = connected("profile-binding-agent-first-use").await;
    let _ = core.shared.store.load(
        json!({"session":{"id":"s"},"history":[],"turns":[{"id":"current","status":"in_progress"}]}),
        vw_session::LoadKind::New,
    ).unwrap();
    super::profile_selection::select_picked(&core.shared, "s", "chrome:work")
        .await
        .unwrap();
    test_support::chat(&core.shared, "other");
    super::profile_selection::select_picked(&core.shared, "other", "chrome:personal")
        .await
        .unwrap();
    let mut events = core.subscribe(None);
    open(&core, "edge:work").await.unwrap();
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("edge:work")
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "other").as_deref(),
        Some("chrome:personal")
    );
    let refusal = open(&core, "chrome:work").await.unwrap_err();
    assert!(
        refusal.contains("Close that browser session first"),
        "{refusal}"
    );
    let mut notices = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_millis(300), events.next()).await
    {
        let value = serde_json::to_value(event).unwrap();
        if value["change"]["params"]["warning"]["code"] == "browser_profile_used_instead" {
            notices.push(value);
        }
    }
    assert_eq!(notices.len(), 1, "one notice after first successful use");
    assert_eq!(notices[0]["change"]["sessionId"], "s");
    assert_eq!(
        notices[0]["change"]["params"]["warning"]["message"],
        "The agent used Edge · Work instead of the picked Chrome · Work."
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
        error.contains("another Vibe Work identity") && error.contains("reconnect"),
        "{error}"
    );
    core.shutdown().await;
}

/// PREDICT: a wrong-profile open reply cannot become first use, change the
/// person's saved pick, or allow another open while its reported session may live.
#[tokio::test]
async fn a_wrong_profile_in_an_open_reply_never_rebinds_the_chat() {
    let core = connected("profile-binding-wrong-reply").await;
    super::profile_selection::select_picked(&core.shared, "s", "chrome:work")
        .await
        .unwrap();
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
    // The helper reported a live session despite the wrong identity. A second profile must wait for proof it ended.
    assert!(open(&core, "edge:work").await.is_err());
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_session")
            .count(),
        1
    );
    assert!(super::profile_bindings::uncertain(
        &core.shared.chrome_control,
        "s"
    ));
    core.shutdown().await;
}

/// PREDICT: an unknown open receipt with a session ID cannot let this chat open
/// a different external profile before that possible session is positively ended;
/// the picker exposes an unconfirmed row even without a prior person pick.
#[tokio::test]
async fn an_unknown_first_open_fences_a_second_profile() {
    let core = connected("profile-binding-unknown-first-open").await;
    let calls = test_support::answering(&core.shared, |tool, _| {
        if tool == "chrome_session" {
            json!({"structuredContent":{"sessionId":"maybe-task","profileId":"chrome:work","status":"unknown"}})
        } else {
            json!({"structuredContent":{}})
        }
    });
    let first = open(&core, "chrome:work").await.unwrap();
    assert_eq!(
        first["vwMcpResult"]["structuredContent"]["status"],
        "unknown"
    );
    let row = super::profile_selection::view(&core.shared.chrome_control)
        .unwrap()
        .into_iter()
        .find(|row| row.session_id == "s")
        .unwrap();
    assert_eq!(row.state, "unconfirmed");
    assert_eq!(row.profile_id, None);
    assert!(open(&core, "edge:work").await.is_err());
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_session")
            .count(),
        1
    );
    assert!(super::profile_bindings::uncertain(
        &core.shared.chrome_control,
        "s"
    ));
    let reloaded = super::profile_bindings::Bindings::load(core.shared.chrome_control.dir());
    assert!(reloaded.check("s", "edge:work").is_err());
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{
            "version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{
                "sessionId":"maybe-task","clientSessionId":"other","profileId":"chrome:work",
                "state":"stopped","capture":"off"
            }]
        }}),
    );
    assert!(super::profile_bindings::uncertain(
        &core.shared.chrome_control,
        "s"
    ));
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{
            "version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{
                "sessionId":"maybe-task","clientSessionId":"s","profileId":"chrome:work",
                "state":"stopped","capture":"off"
            }]
        }}),
    );
    assert!(!super::profile_bindings::uncertain(
        &core.shared.chrome_control,
        "s"
    ));
    core.shutdown().await;
}
