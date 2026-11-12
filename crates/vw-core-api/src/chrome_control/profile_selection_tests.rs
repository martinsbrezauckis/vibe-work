//! Chat selection regressions through person commands and the agent gateway, with acknowledged helper releases.
use super::{edge_agent_tests::fixture, gateway, test_support};
use serde_json::{Value, json};

fn profiles() -> Value {
    json!([
        {"profileId":"chrome:work","name":"Work","connected":true},
        {"profileId":"chrome:personal","name":"Personal","connected":true}
    ])
}
async fn command(core: &crate::Core, request: Value) -> Result<crate::CommandResult, String> {
    let command = serde_json::from_value(request).map_err(|error| error.to_string())?;
    core.chrome_control(crate::Origin::Ui, command)
        .await
        .map_err(|error| error.message)
}
async fn choose(core: &crate::Core, profile: &str) -> Result<crate::CommandResult, String> {
    command(
        core,
        json!({"op":"selectProfile","sessionId":"s","profileId":profile}),
    )
    .await
}
#[allow(clippy::unwrap_used, reason = "test-only state assertion")]
fn selected(core: &crate::Core) -> Option<String> {
    core.shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap()
        .get("s")
        .cloned()
}

#[tokio::test]
async fn proven_deleted_profile_releases_its_old_binding_without_an_impossible_ack() {
    let core = fixture("profile-choice-deleted-release", profiles()).await;
    choose(&core, "chrome:work").await.unwrap();
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let available = json!([{"profileId":"chrome:personal","name":"Personal","connected":true}]);
    test_support::observe_status(&core.shared, &json!({"profiles":available}));
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut("chrome:work")
        .unwrap()
        .deleted = true;
    let seen = test_support::answering(&core.shared, move |tool, args| {
        if tool == "chrome_status" {
            return json!({"structuredContent":{"profiles":available}});
        }
        if args["action"] == "open" {
            return json!({"structuredContent":{"sessionId":"new-task","profileId":"chrome:personal","state":"active"}});
        }
        json!({"structuredContent":{"error":{"code":"PROFILE_OFFLINE","message":"The deleted profile is offline. Select another profile."}}})
    });
    choose(&core, "chrome:personal")
        .await
        .expect("positive browser deletion must allow selecting a surviving profile");
    assert_eq!(selected(&core).as_deref(), Some("chrome:personal"));
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["sessionId"] == "task"),
        "deleted runtime cannot acknowledge old leases"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn several_profiles_require_a_person_choice_even_when_the_agent_supplies_an_exact_id() {
    let core = fixture("profile-choice-required", profiles()).await;
    let error = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:work"}),
    )
    .await
    .expect_err("an agent cannot select an account from multiple choices");
    assert!(
        error.contains("Work") && error.contains("Personal") && error.contains("user"),
        "{error}"
    );
    assert_eq!(selected(&core), None);
    core.shutdown().await;
}

#[tokio::test]
async fn default_seeds_only_an_unbound_chat() {
    let core = fixture("profile-choice-default", profiles()).await;
    command(
        &core,
        json!({"op":"setDefaultProfile","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome"}),
    )
    .await
    .unwrap();
    assert_eq!(selected(&core).as_deref(), Some("chrome:work"));
    command(
        &core,
        json!({"op":"setDefaultProfile","profileId":"chrome:personal"}),
    )
    .await
    .unwrap();
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome"}),
    )
    .await
    .unwrap();
    assert_eq!(selected(&core).as_deref(), Some("chrome:work"));
    core.shutdown().await;
}

#[tokio::test]
async fn a_person_switch_fences_before_release_and_binds_only_after_acknowledgement() {
    let core = fixture("profile-choice-release", profiles()).await;
    choose(&core, "chrome:work").await.unwrap();
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let shared = core.shared.clone();
    let seen = test_support::answering(&core.shared, move |tool, args| {
        if args["action"] == "open" {
            assert_eq!(
                shared
                    .chrome_control
                    .chat_profiles
                    .lock()
                    .unwrap()
                    .get("s")
                    .map(String::as_str),
                Some("chrome:personal")
            );
            return json!({"structuredContent":{"sessionId":"new-task","profileId":"chrome:personal","state":"active"}});
        }
        if tool == "chrome_status" {
            return json!({"structuredContent":{"profiles":profiles()}});
        }
        assert!(
            !shared
                .chrome_control
                .tasks
                .lock()
                .unwrap()
                .contains_key("task"),
            "old lease must be fenced before stop delivery"
        );
        assert_eq!(
            shared
                .chrome_control
                .chat_profiles
                .lock()
                .unwrap()
                .get("s")
                .map(String::as_str),
            Some("chrome:work")
        );
        assert_eq!(tool, "chrome_session");
        assert_eq!(args["sessionId"], "task");
        json!({"structuredContent":{"sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"off","desiredState":"stopped"}})
    });
    choose(&core, "chrome:personal").await.unwrap();
    assert_eq!(selected(&core).as_deref(), Some("chrome:personal"));
    assert!(!seen.lock().unwrap().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn unknown_release_keeps_the_old_binding_and_blocks_new_work() {
    let core = fixture("profile-choice-unknown", profiles()).await;
    choose(&core, "chrome:work").await.unwrap();
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"status":"unknown"}}),
    );
    let error = choose(&core, "chrome:personal").await.unwrap_err();
    assert!(error.contains("unconfirmed"), "{error}");
    assert_eq!(selected(&core).as_deref(), Some("chrome:work"));
    assert!(
        gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"chrome:work"})
        )
        .await
        .is_err()
    );
    let view = serde_json::to_value(super::commands::view(&core.shared)).unwrap();
    assert_eq!(view["chatProfiles"][0]["state"], "unconfirmed");
    let restored = super::profile_bindings::Bindings::load(core.shared.chrome_control.dir());
    assert!(
        restored
            .check("s", "chrome:work")
            .unwrap_err()
            .contains("unconfirmed")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_persons_exact_tab_binds_its_verified_profile_even_when_another_is_default() {
    let core = fixture("profile-choice-tab", profiles()).await;
    command(
        &core,
        json!({"op":"setDefaultProfile","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"requestId":"picked","status":"completed","data":{"url":"https://example.test","title":"Page","text":"Selected"}}}),
    );
    super::tabs::pick(
        &core.shared,
        &core.shared.chrome_control,
        "chrome:personal",
        7,
        Some("s"),
    )
    .await
    .unwrap();
    assert_eq!(selected(&core).as_deref(), Some("chrome:personal"));
    core.shutdown().await;
}

#[tokio::test]
async fn an_existing_single_helper_binding_migrates_without_following_the_new_default() {
    let core = fixture("profile-legacy-single", profiles()).await;
    command(
        &core,
        json!({"op":"setDefaultProfile","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"profiles":profiles(),"sessions":[{"sessionId":"old-task","clientSessionId":"s","profileId":"chrome:personal","state":"active"}]}}),
    );
    let mut args = json!({"action":"open","profileId":"chrome"});
    super::session_title::apply(
        &core.shared,
        "s",
        super::agent_browser::Browser::Chrome,
        &mut args,
    )
    .await
    .unwrap();
    assert_eq!(args["profileId"], "chrome:personal");
    core.shutdown().await;
}
#[tokio::test]
async fn ambiguous_legacy_tasks_require_a_person_even_with_only_one_connected_profile() {
    let core = fixture("profile-legacy-multiple", profiles()).await;
    test_support::answering(&core.shared, |_, _| {
        json!({"structuredContent":{"profiles":[{"profileId":"chrome:work","name":"Work","connected":true}],"sessions":[
            {"sessionId":"old-work","clientSessionId":"s","profileId":"chrome:work","state":"active"},
            {"sessionId":"old-personal","clientSessionId":"s","profileId":"chrome:personal","state":"disconnected"}
        ]}})
    });
    let mut args = json!({"action":"open","profileId":"chrome"});
    let error = super::session_title::apply(
        &core.shared,
        "s",
        super::agent_browser::Browser::Chrome,
        &mut args,
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("user") && error.contains("profile"),
        "{error}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn switching_off_a_profile_during_a_bound_tab_read_discards_that_read() {
    let core = fixture("profile-tab-off", profiles()).await;
    choose(&core, "chrome:work").await.unwrap();
    let shared = core.shared.clone();
    test_support::answering(&core.shared, move |_, _| {
        let mut profiles = shared.chrome_control.profiles.lock().unwrap();
        profiles.records.get_mut("chrome:work").unwrap().enabled = false;
        *profiles
            .generations
            .entry("chrome:work".into())
            .or_default() += 1;
        json!({"structuredContent":{"requestId":"read","status":"completed","data":{"url":"https://page.test/","title":"Page","text":"Read after Off"}}})
    });
    let result = super::tabs::pick(
        &core.shared,
        &core.shared.chrome_control,
        "chrome:work",
        7,
        Some("s"),
    )
    .await;
    assert!(
        result.is_err(),
        "an Off profile cannot deliver a late tab read: {result:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn debug_api_cannot_change_a_chats_account() {
    let core = fixture("profile-person-only", profiles()).await;
    let result = core
        .chrome_control(
            crate::Origin::DebugApi,
            super::ChromeControlRequest::SelectProfile {
                session_id: "s".into(),
                profile_id: "chrome:work".into(),
            },
        )
        .await;
    assert!(result.unwrap_err().message.contains("user"));
    assert_eq!(selected(&core), None);
    core.shutdown().await;
}

#[tokio::test]
async fn debug_tab_context_cannot_bypass_the_persons_profile_selection() {
    let core = fixture("profile-debug-tab", profiles()).await;
    test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"requestId":"read","status":"completed","data":{"url":"https://page.test/","title":"Page","text":"Read"}}}),
    );
    let result = core
        .chrome_control(
            crate::Origin::DebugApi,
            super::ChromeControlRequest::TabContext {
                session_id: Some("s".into()),
                profile_id: "chrome:work".into(),
                tab_id: 7,
            },
        )
        .await;
    assert!(result.unwrap_err().message.contains("user"));
    assert_eq!(selected(&core), None);
    core.shutdown().await;
}
