//! Turning a profile or a whole browser off revokes its admitted leases, including a lease admitted while Off was
//! being applied and leases whose display rows were evicted, without revoking sibling profiles or other browsers.
use super::{agent_browser::Browser, edge_agent_tests::fixture, test_support};
use serde_json::json;

#[tokio::test]
async fn profile_off_between_reply_validation_and_binding_cannot_leave_a_live_lease() {
    let core = fixture(
        "profile-off-binding-race",
        json!([
            {"profileId":"chrome:work","name":"Work","connected":true}
        ]),
    )
    .await;
    let shared = &core.shared;
    let pending = super::person_stop::PendingCall::new(
        shared,
        "s",
        "chrome_session",
        &json!({"action":"open","profileId":"chrome:work"}),
    );
    let mut answer = super::owner::Answer::Helper(
        json!({"structuredContent":{"sessionId":"late","profileId":"chrome:work","state":"active"}}),
    );
    pending
        .finish(shared, "s", Browser::Chrome, &answer)
        .unwrap();
    test_support::answering(shared, |_, _| json!({"applied":true}));
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::SetProfileEnabled {
            profile_id: "chrome:work".into(),
            enabled: false,
        },
    )
    .await
    .unwrap();
    assert!(
        super::gateway::scope_result(
            shared,
            "s",
            "chrome_session",
            true,
            Browser::Chrome,
            &pending,
            &mut answer
        )
        .is_err()
    );
    assert!(
        !shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .contains_key("late")
    );
    core.shutdown().await;
}

#[allow(
    clippy::unwrap_used,
    reason = "test fixture must admit its exact profile leases"
)]
async fn off_fences_evicted_leases(browser_wide: bool) {
    let profiles = json!([
        {"profileId":"chrome:work","name":"Work","connected":true},
        {"profileId":"chrome:personal","name":"Personal","connected":true},
        {"profileId":"edge:work","name":"Work","connected":true}
    ]);
    let core = fixture(
        if browser_wide {
            "browser-off-evicted-leases"
        } else {
            "profile-off-evicted-leases"
        },
        profiles.clone(),
    )
    .await;
    test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_status" => json!({"structuredContent":{"profiles":profiles,"sessions":[]}}),
        "chrome_session" => {
            json!({"structuredContent":{"sessionId":args["profileId"],"profileId":args["profileId"],"state":"active"}})
        }
        _ => json!({"applied":true}),
    });
    for (chat, profile) in [
        ("s", "chrome:work"),
        ("personal", "chrome:personal"),
        ("edge", "edge:work"),
    ] {
        if chat != "s" {
            test_support::chat(&core.shared, chat);
        }
        super::profile_selection::select(&core.shared, chat, profile)
            .await
            .unwrap();
        super::gateway::call(
            &core.shared,
            chat,
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .unwrap();
    }
    let chrome = &core.shared.chrome_control;
    {
        let mut work = chrome.work.lock().unwrap();
        for index in 0..129 {
            work.open(
                &format!("history-{index}"),
                "history",
                "stopped",
                "edge:old",
                None,
            );
        }
        assert_eq!(work.profile_of("chrome:work"), None);
        assert_eq!(work.profile_of("chrome:personal"), None);
        assert_eq!(work.profile_of("edge:work"), None);
    }
    if browser_wide {
        core.browser_enabled(Browser::Chrome, false).await.unwrap();
    } else {
        core.chrome_control(
            crate::Origin::Ui,
            super::ChromeControlRequest::SetProfileEnabled {
                profile_id: "chrome:work".into(),
                enabled: false,
            },
        )
        .await
        .unwrap();
    }
    {
        let tasks = chrome.tasks.lock().unwrap();
        assert!(
            !tasks.contains_key("chrome:work"),
            "Off must revoke the exact admitted lease even when its display row was evicted"
        );
        assert_eq!(tasks.contains_key("chrome:personal"), !browser_wide);
        assert!(
            tasks.contains_key("edge:work"),
            "the sibling browser remains authorized"
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn profile_off_revokes_evicted_leases_without_revoking_sibling_profiles() {
    off_fences_evicted_leases(false).await;
}

#[tokio::test]
async fn browser_off_revokes_evicted_leases_without_revoking_other_browsers() {
    off_fences_evicted_leases(true).await;
}
