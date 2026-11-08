//! Independent browser switches preserve legacy choices and fence only the selected browser's work.
//! Uses isolated core state and helper doubles; no native browser is opened.
use super::{agent_browser::Browser, *};
use serde_json::json;

#[tokio::test]
async fn legacy_switch_migrates_to_both_browsers_then_changes_independently() {
    for legacy in [false, true] {
        let core =
            crate::offline_core::start_with(&format!("browser-switch-{legacy}"), |config, root| {
                config.chrome_control.package = Some(root.join("package"));
                config.chrome_control.user_home = Some(root.join("home"));
                let dir = root.join("home/VibeWork/ChromeControl");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("enabled.json"), legacy.to_string()).unwrap();
            })
            .await;
        let chrome = &core.shared.chrome_control;
        assert_eq!(Browser::Chrome.enabled(&core.shared), legacy);
        assert_eq!(Browser::Edge.enabled(&core.shared), legacy);
        chrome.set_enabled(!legacy).unwrap();
        assert_eq!(Browser::Chrome.enabled(&core.shared), !legacy);
        assert_eq!(
            Browser::Edge.enabled(&core.shared),
            legacy,
            "Chrome must not change Edge"
        );
        let restored = ChromeControl::new(std::path::Path::new("unused"), chrome.config.clone());
        let restored_view = serde_json::to_value(commands::view(&core.shared)).unwrap();
        assert_eq!(restored.enabled(), !legacy);
        assert_eq!(restored.edge_enabled(), legacy);
        assert_eq!(restored_view["edgeEnabled"], legacy);
        core.shutdown().await;
    }
}

#[tokio::test]
async fn edge_switch_is_a_person_command_and_preserves_chrome() {
    let core = crate::offline_core::start_with("edge-independent-command", |config, root| {
        config.chrome_control.package = Some(root.join("package"));
    })
    .await;
    let request: commands::ChromeControlRequest = serde_json::from_value(json!({
        "op":"setEdgeEnabled", "enabled":false
    }))
    .expect("Edge has its own switch command");
    let result = core
        .execute(crate::Origin::Ui, crate::Command::ChromeControl { request })
        .await
        .unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["view"]["enabled"], true);
    assert_eq!(value["view"]["edgeEnabled"], false);
    assert!(Browser::Chrome.enabled(&core.shared));
    assert!(!Browser::Edge.enabled(&core.shared));
    core.shutdown().await;
}

#[tokio::test]
async fn chrome_off_fences_its_work_and_late_replies_but_preserves_edge() {
    let core = crate::offline_core::start_with("browser-switch-work", |config, root| {
        config.chrome_control.package = Some(root.join("package"));
    })
    .await;
    let shared = &core.shared;
    let chrome = &shared.chrome_control;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let calls = test_support::answering(shared, |_, _| json!({"applied":true}));
    for (id, profile) in [("chrome-work", "chrome:p"), ("edge-work", "edge:p")] {
        chrome
            .work
            .lock()
            .unwrap()
            .open(id, "s", "active", profile, None);
        chrome.tasks.lock().unwrap().insert(id.into(), "s".into());
    }
    let pending = person_stop::PendingCall::new(
        shared,
        "s",
        "chrome_session",
        &json!({"action":"open","profileId":"chrome:p"}),
    );
    core.browser_enabled(Browser::Chrome, false).await.unwrap();
    assert_eq!(
        chrome.work.lock().unwrap().state_of("chrome-work", "s"),
        Some("stopping")
    );
    assert_eq!(
        chrome.work.lock().unwrap().state_of("edge-work", "s"),
        Some("active")
    );
    assert!(chrome.owner.is_running(), "Edge retains the shared helper");
    assert_eq!(
        agent_browser::selected(shared, "unbound", "chrome_status", &json!({})),
        Browser::Edge
    );
    core.browser_enabled(Browser::Chrome, true).await.unwrap();
    let late = owner::Answer::Helper(
        json!({"structuredContent":{"sessionId":"late","profileId":"chrome:p"}}),
    );
    assert!(
        pending
            .finish(shared, "s", Browser::Chrome, &late)
            .unwrap_err()
            .contains("stopped by its switch")
    );
    pending.finish(shared, "s", Browser::Edge, &late).unwrap();
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|(method, args)| method == "owner.browserAvailability"
                && args["chrome"] == false
                && args["edge"] == true)
    );
    core.shutdown().await;
}

#[test]
fn corrupt_per_browser_preferences_fail_closed_without_reusing_legacy_on() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("enabled.json"), "true").unwrap();
    std::fs::write(root.path().join("browser-enabled.json"), "broken").unwrap();
    let switches = store::Switches::load(Some(root.path()));
    assert!(!switches.chrome && !switches.edge);
}
