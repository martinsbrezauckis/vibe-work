//! Regression tests for `safari.rs`; uses the parent module and is called by Cargo tests.
use super::*;

#[test]
fn safari_requires_the_containing_apps_release_version() {
    assert_eq!(
        diagnose(true, true, Some(crate::version().into())).state,
        AddonState::Connected
    );
    assert_eq!(
        diagnose(true, true, Some("0.1.8".into())).state,
        AddonState::UpdateNeeded
    );
    assert_eq!(
        diagnose(true, true, Some("65535.0.0".into())).state,
        AddonState::Connected
    );
    assert_eq!(
        diagnose(true, true, Some("unknown".into())).state,
        AddonState::UpdateNeeded
    );
}

#[cfg(feature = "your-safari")]
#[tokio::test]
async fn safari_disable_saves_and_delivers_its_switch_without_stopping_other_browsers() {
    let core = crate::offline_core::start_with("safari-switch-delivery", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let chrome = &core.shared.chrome_control;
    let dir = chrome.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    let calls = chrome.owner.answering(|_, _| json!({"applied":true}));
    core.safari_enabled_held(false).await.unwrap();
    assert_eq!(
        std::fs::read(dir.join("safari-enabled.json")).unwrap(),
        b"false"
    );
    assert!(!enabled(&core.shared));
    assert!(chrome.enabled() && chrome.edge_enabled());
    assert!(chrome.owner.is_running());
    assert!(calls.lock().unwrap().iter().any(|(method, params)| {
        method == "owner.browserAvailability"
            && params["safari"] == false
            && params["chrome"] == true
            && params["generation"] == 1
    }));
    core.shutdown().await;
}

#[cfg(feature = "your-safari")]
#[tokio::test]
async fn safari_pairing_start_does_not_require_chromes_switch() {
    let core = crate::offline_core::start_with("safari-independent-switch", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let chrome = &core.shared.chrome_control;
    chrome.set_enabled(false).unwrap();
    let dir = chrome.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    // No test keyring: reaching that check proves independent start admission without launching a helper.
    assert!(super::super::owner::start(&core.shared).await.is_err());
    assert!(!chrome.enabled());
    core.shutdown().await;
}

#[test]
fn safari_connection_labels_and_actions_match_the_shared_browser_model() {
    for (on, version) in [
        (false, None),
        (true, None),
        (true, Some("0.1.5")),
        (true, Some(crate::version())),
    ] {
        let actual = diagnose(true, on, version.map(str::to_owned));
        let expected = super::super::addon::view_for(
            actual.state,
            "",
            super::super::agent_browser::Browser::Safari,
        );
        assert_eq!(actual.label, expected.label);
        assert_eq!(actual.action, expected.action);
    }
}

#[test]
fn safari_has_a_separate_remove_command() {
    let request = serde_json::from_value::<super::super::commands::ChromeControlRequest>(
        json!({"op":"removeSafari"}),
    );
    assert!(
        request.is_ok(),
        "Safari removal cannot remove Chrome or Edge: {request:?}"
    );
}

#[test]
fn missing_off_unpaired_outdated_and_paired_remain_distinct() {
    assert_eq!(diagnose(false, true, None).state, AddonState::Unavailable);
    assert_eq!(diagnose(true, false, None).state, AddonState::SwitchedOff);
    assert_eq!(diagnose(true, true, None).state, AddonState::NotPaired);
    assert_eq!(
        diagnose(true, true, Some("0.1.5".into())).state,
        AddonState::UpdateNeeded
    );
    let paired = diagnose(true, true, Some(crate::version().into()));
    assert_eq!(paired.state, AddonState::Connected);
    assert!(paired.instruction.contains("website permission"));
    assert!(paired.instruction.contains("read the page"));
}
