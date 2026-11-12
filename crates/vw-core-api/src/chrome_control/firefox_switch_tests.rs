//! Firefox availability must fence its own work and survive restart without changing Chrome or Edge.
use super::{agent_browser::Browser, store};
#[tokio::test]
async fn firefox_switch_preserves_other_browsers_and_its_saved_choice() {
    let core = crate::offline_core::start_with("firefox-switch", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let chrome = &core.shared.chrome_control;
    let chrome_generation = chrome.browser_generation(Browser::Chrome);
    chrome.set_browser_enabled(Browser::Firefox, false).unwrap();
    assert!(Browser::Chrome.enabled(&core.shared));
    assert!(Browser::Edge.enabled(&core.shared));
    assert!(!Browser::Firefox.enabled(&core.shared));
    assert_eq!(
        chrome.browser_generation(Browser::Chrome),
        chrome_generation
    );
    let saved = serde_json::to_value(store::Switches::load(chrome.dir())).unwrap();
    assert_eq!(saved["firefox"], false);
    assert_eq!(saved["chrome"], true);
    core.shutdown().await;
}

#[tokio::test]
async fn firefox_removal_clears_live_facts_and_rejects_late_status() {
    let core = crate::offline_core::start_with("firefox-remove-facts", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let chrome = &core.shared.chrome_control;
    chrome.owner.answering(|name, _| match name {
        "chrome_status" => {
            serde_json::json!({"structuredContent":{"firefoxWindow":"Window closed"}})
        }
        "owner.browserUnpair" => serde_json::json!({"unpaired":true}),
        "owner.browserAvailability" => serde_json::json!({"applied":true}),
        _ => serde_json::Value::Null,
    });
    chrome
        .owner
        .connected_profiles
        .observe(&serde_json::json!({"profiles":[
        {"profileId":"firefox:profile","connected":true,"extensionVersion":"0.1.7"},
        {"profileId":"chrome:profile","connected":true,"extensionVersion":"0.1.7"}]}));
    core.firefox_remove().await.unwrap();
    assert_eq!(chrome.owner.connected_profiles.snapshot().len(), 1);
    let mut late =
        serde_json::json!({"profiles":[{"profileId":"firefox:profile","connected":true}]});
    super::remove::filter_observation(&core.shared, &mut late);
    assert_eq!(late["profiles"], serde_json::json!([]));
    core.shutdown().await;
}

#[tokio::test]
async fn firefox_only_registration_starts_the_shared_owner() {
    let registry = super::registry_native_tests::TestRoot::new("firefox-owner-start");
    let root = format!(r"{}\FirefoxNativeMessagingHosts", registry.key());
    let core = crate::offline_core::start_with("firefox-only-owner", move |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        config.chrome_control.user_home = Some(dir.join("home"));
        config.chrome_control.firefox_registry_root = root;
        config.chrome_control.macos = Some(false);
    })
    .await;
    let chrome = &core.shared.chrome_control;
    let package = chrome.config.package.as_ref().unwrap().join("bin");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join(super::setup::helper_name()), b"fixture").unwrap();
    super::setup::install_for(
        &chrome.config,
        chrome.dir().unwrap(),
        &core.shared.instructions.own_home,
        "Mozilla Firefox",
    )
    .unwrap();
    assert!(core.shared.config.keys.is_none());
    let result = super::owner::start(&core.shared).await;
    assert!(
        result.is_err(),
        "Registered Firefox must attempt owner startup and diagnose the missing key service"
    );
    super::setup::remove_browser(
        &chrome.config,
        chrome.dir().unwrap(),
        "Mozilla Firefox",
        true,
    )
    .unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn pending_firefox_call_tracks_its_own_switch_generation() {
    use super::person_stop::PendingCall;
    use serde_json::json;
    let core = crate::offline_core::start("firefox-pending-switch").await;
    let chrome = &core.shared.chrome_control;
    let capture = || PendingCall::new(&core.shared, "chat", "chrome_execute", &json!({}));
    chrome.set_browser_enabled(Browser::Chrome, false).unwrap();
    let pending = capture();
    assert!(
        pending
            .refusal(&core.shared, "chat", Browser::Firefox)
            .is_none()
    );
    chrome.set_browser_enabled(Browser::Chrome, true).unwrap();
    assert!(
        pending
            .refusal(&core.shared, "chat", Browser::Firefox)
            .is_none()
    );
    let pending = capture();
    chrome.set_browser_enabled(Browser::Firefox, false).unwrap();
    chrome.set_browser_enabled(Browser::Firefox, true).unwrap();
    assert!(
        pending
            .refusal(&core.shared, "chat", Browser::Firefox)
            .unwrap()
            .contains("switch")
    );
    assert!(
        pending
            .refusal(&core.shared, "chat", Browser::Chrome)
            .is_none()
    );
    core.shutdown().await;
}
