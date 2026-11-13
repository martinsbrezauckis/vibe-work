//! Retained profile identity under missing folders, damaged settings and extension reinstall; no native browser.
use super::{agent_browser::Browser, edge_agent_tests::fixture, test_support};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn a_sole_folder_and_uuid_do_not_prove_a_profile_association() {
    let core = super::browser_start_tests::fixture(
        "profile-no-folder-guess",
        Browser::Chrome,
        Arc::new(|_| Ok(())),
    )
    .await;
    let chrome = &core.shared.chrome_control;
    let folder = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("browser-data/Default");
    assert!(
        !super::profile_folders::load(chrome, Browser::Chrome).contains(&folder),
        "the frozen hello contains no filesystem association"
    );
    assert_eq!(
        super::installation::paired_directory(&core.shared, Browser::Chrome),
        None,
        "an installed folder must not select the account"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn unreadable_saved_bindings_cannot_reset_a_chat_to_another_default() {
    let core = fixture("profile-corrupt-bindings", json!([])).await;
    let chrome = &core.shared.chrome_control;
    std::fs::write(chrome.dir().unwrap().join("chat-profiles.json"), b"{broken").unwrap();
    let restored = super::profile_bindings::Bindings::load(chrome.dir());
    assert!(
        restored.check("old-chat", "chrome:other").is_err(),
        "damaged bindings must fail closed"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn unreadable_profile_settings_cannot_switch_every_identity_back_on() {
    let core = fixture("profile-corrupt-settings", json!([])).await;
    let chrome = &core.shared.chrome_control;
    std::fs::write(
        chrome.dir().unwrap().join("browser-profiles.json"),
        b"{broken",
    )
    .unwrap();
    let restored = super::profiles::Profiles::load(chrome.dir());
    assert!(
        !restored.enabled("chrome:work"),
        "a lost Off setting must not grant access"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn removing_a_pairing_does_not_permanently_block_its_next_approval_ceremony() {
    let core = fixture(
        "profile-removed-repair",
        json!([{"profileId":"chrome:work","name":"Work","connected":true}]),
    )
    .await;
    test_support::answering(&core.shared, |tool, _| match tool {
        "owner.profileUnpair" => json!({"unpaired":true}),
        _ => json!({"applied":true}),
    });
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::RemoveProfile {
            profile_id: "chrome:work".into(),
        },
    )
    .await
    .unwrap();
    let chrome = &core.shared.chrome_control;
    assert!(
        !chrome
            .profiles
            .lock()
            .unwrap()
            .disabled()
            .contains(&"chrome:work".into()),
        "unpair removes the trust pin; a new person approval must remain possible"
    );
    assert_eq!(
        super::profiles::view(&core.shared),
        [] as [super::BrowserProfile; 0]
    );
    assert!(
        super::profiles::check(chrome, "chrome:work").is_err(),
        "removed identity remains unavailable to actions"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn renamed_and_reinstalled_profiles_keep_distinct_history_and_binding() {
    let core = fixture(
        "profile-renamed-reinstalled",
        json!([{"profileId":"chrome:old","name":"Work","connected":true}]),
    )
    .await;
    let chrome = &core.shared.chrome_control;
    super::profile_bindings::remember(chrome, "s", "chrome:old", None).unwrap();
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::SetDefaultProfile {
            profile_id: "chrome:old".into(),
        },
    )
    .await
    .unwrap();
    test_support::observe_status(
        &core.shared,
        &json!({"profiles":[{"profileId":"chrome:old","name":"Business","connected":true}]}),
    );
    assert_eq!(super::profiles::view(&core.shared)[0].name, "Business");
    test_support::observe_status(
        &core.shared,
        &json!({"profiles":[{"profileId":"chrome:new","name":"Business","connected":true}]}),
    );
    let rows = super::profiles::view(&core.shared);
    assert_eq!(rows.len(), 2);
    let old = rows
        .iter()
        .find(|row| row.profile_id == "chrome:old")
        .unwrap();
    assert_eq!(old.state, super::ProfileState::Offline);
    assert!(!old.eligible);
    assert_eq!(
        super::profile_bindings::selected(chrome, "s").as_deref(),
        Some("chrome:old")
    );
    assert!(super::profile_bindings::check(chrome, "s", "chrome:new").is_err());
    core.shutdown().await;
}

#[tokio::test]
async fn restarting_cannot_assign_an_existing_receipt_id_to_another_chat() {
    let core = fixture("profile-session-chat-restart", json!([])).await;
    let chrome = &core.shared.chrome_control;
    super::profile_bindings::remember(chrome, "old", "chrome:work", Some("receipt")).unwrap();
    *chrome.bindings.lock().unwrap() = super::profile_bindings::Bindings::load(chrome.dir());
    assert!(
        super::profile_bindings::remember(chrome, "new", "chrome:work", Some("receipt")).is_err()
    );
    core.shutdown().await;
}

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

#[tokio::test]
async fn a_changed_owner_cannot_acknowledge_the_previous_owners_release() {
    let core = fixture(
        "profile-release-owner-change",
        json!([
            {"profileId":"chrome:work","connected":true},
            {"profileId":"chrome:personal","connected":true}
        ]),
    )
    .await;
    let chrome = &core.shared.chrome_control;
    test_support::observe_status(&core.shared, &json!({"ownerPublicKey":"old-owner"}));
    super::profile_bindings::remember(chrome, "s", "chrome:work", Some("old-task")).unwrap();
    test_support::observe_status(&core.shared, &json!({"ownerPublicKey":"new-owner"}));
    let calls = test_support::answering(&core.shared, |_, _| {
        json!({"structuredContent":{
            "sessionId":"old-task","profileId":"chrome:work","state":"stopped","capture":"off","ownerPublicKey":"new-owner"
        }})
    });
    assert!(
        super::profile_selection::select(&core.shared, "s", "chrome:personal")
            .await
            .is_err()
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "a new owner cannot stop the previous owner's task"
    );
    assert_eq!(
        super::profile_bindings::selected(chrome, "s").as_deref(),
        Some("chrome:work")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn agent_settings_commands_cannot_select_the_next_chats_account() {
    let core = fixture(
        "profile-default-person-only",
        json!([
            {"profileId":"chrome:work","connected":true},
            {"profileId":"chrome:personal","connected":true}
        ]),
    )
    .await;
    assert!(
        core.chrome_control(
            crate::Origin::DebugApi,
            super::ChromeControlRequest::SetDefaultProfile {
                profile_id: "chrome:personal".into()
            }
        )
        .await
        .is_err()
    );
    assert!(
        super::profiles::view(&core.shared)
            .iter()
            .all(|row| !row.is_default)
    );
    assert!(core.shared.window_drive.set(Box::new(|| true)).is_ok());
    assert!(
        core.chrome_control(
            crate::Origin::Ui,
            super::ChromeControlRequest::SetDefaultProfile {
                profile_id: "chrome:personal".into()
            }
        )
        .await
        .is_err()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_profile_rename_publishes_while_the_browser_stays_connected() {
    let core = fixture(
        "profile-rename-event",
        json!([
            {"profileId":"chrome:work","name":"Work","connected":true},
            {"profileId":"chrome:personal","name":"Personal","connected":true}
        ]),
    )
    .await;
    let mut events = core.shared.events.subscribe();
    test_support::observe_status(
        &core.shared,
        &json!({"profiles":[
            {"profileId":"chrome:work","name":"Business","connected":true},
            {"profileId":"chrome:personal","name":"Personal","connected":true}
        ]}),
    );
    let mut published = false;
    while let Ok(event) = events.try_recv() {
        if let crate::CoreEvent::ChromeControl { view } = event {
            published |= view
                .paired_profiles
                .as_ref()
                .is_some_and(|rows| rows.iter().any(|row| row.name == "Business"));
        }
    }
    assert!(
        published,
        "the chat picker must receive profile changes without reopening Settings"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn final_review_receipts_keep_identity_after_display_history_eviction() {
    let core = fixture(
        "profile-receipt-display-eviction",
        json!([
            {"profileId":"chrome:work","name":"Work","connected":true}
        ]),
    )
    .await;
    super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    {
        let mut work = core.shared.chrome_control.work.lock().unwrap();
        for index in 0..129 {
            work.open(
                &format!("other-{index}"),
                "other",
                "stopped",
                "edge:personal",
                None,
            );
        }
        assert_eq!(work.profile_of("task"), None);
    }
    let receipt = super::gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"task","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    assert_eq!(
        receipt["vwMcpResult"]["structuredContent"]["profileId"],
        "chrome:work"
    );
    assert_eq!(
        receipt["vwMcpResult"]["structuredContent"]["profileName"],
        "Work"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn final_review_changed_owners_do_not_inherit_the_previous_pairings_default() {
    let core = fixture("profile-default-owner-change", json!([])).await;
    let profiles = json!([
        {"profileId":"chrome:work","name":"Work","connected":true},
        {"profileId":"chrome:personal","name":"Personal","connected":true}
    ]);
    test_support::observe_status(
        &core.shared,
        &json!({"ownerPublicKey":"first-owner","profiles":profiles}),
    );
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::SetDefaultProfile {
            profile_id: "chrome:work".into(),
        },
    )
    .await
    .unwrap();
    test_support::observe_status(
        &core.shared,
        &json!({"ownerPublicKey":"changed-owner","profiles":profiles}),
    );
    assert!(
        super::profiles::view(&core.shared)
            .iter()
            .all(|row| !row.is_default)
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

/// PREDICT (written before the fix): fails. After Settings > Browser removes the only Chrome profile's pairing, the
/// extension is still on in Chrome, but the card takes the "lost connection" branch of a once-paired browser and says
/// "Turn on in Chrome" with no button. Expected: "Not connected" with the Connect action, whose click lets Chrome ask
/// for a new pairing (the native "Connect Chrome to Vibe Work?" approval).
#[tokio::test]
async fn removing_the_only_pairing_offers_connect_instead_of_turn_on() {
    let core = fixture(
        "profile-removed-card",
        json!([{"profileId":"chrome:work","name":"Work","connected":true}]),
    )
    .await;
    test_support::answering(&core.shared, |tool, _| match tool {
        "owner.profileUnpair" => json!({"unpaired":true}),
        _ => json!({"applied":true}),
    });
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::RemoveProfile {
            profile_id: "chrome:work".into(),
        },
    )
    .await
    .unwrap();
    // The helper still runs and the extension is still on, but it has no pairing any more: the once-paired browser
    // reads as connecting (the state the Mac run showed as "Turn on in Chrome").
    test_support::observe_status(&core.shared, &json!({"profiles":[]}));
    let chrome = &core.shared.chrome_control;
    assert!(Browser::Chrome.was_paired(&core.shared));
    chrome.owner.pairing.lock().unwrap().0 = super::PairingState::Connecting;
    let card = super::commands::addon(&core.shared);
    assert_eq!(card.label, "Not connected", "{card:?}");
    assert_eq!(
        card.action,
        Some(super::addon::AddonAction::Connect),
        "{card:?}"
    );
    assert!(!card.instruction.contains("Turn on"), "{card:?}");
    core.shutdown().await;
}
