//! Pairing before Chrome writes its extension directory, and later profile discovery on the status poll.
//! Uses an offline core and disposable package/home; never opens Chrome or a keyring.
use super::*;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn pairing_is_added_before_the_folder_and_later_profiles_emit_an_event() {
    let core = crate::offline_core::start_with("chrome-pairing-discovery", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        config.chrome_control.user_home = Some(dir.join("home"));
    })
    .await;
    let shared = &core.shared;
    let chrome = &shared.chrome_control;
    let package = chrome.config.package.as_ref().unwrap();
    std::fs::create_dir_all(package.join("bin")).unwrap();
    std::fs::write(
        package.join("bin").join(setup::helper_name()),
        b"test helper",
    )
    .unwrap();
    let dir = chrome.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let data = dir.join("chrome-data");
    std::fs::write(
        dir.join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome"],"userData":[data]}).to_string(),
    )
    .unwrap();
    let reply =
        json!({"structuredContent":{"profiles":[{"connected":true}],"ownerFingerprint":"test"}});
    owner::update_pairing(&Arc::downgrade(shared), &reply);
    let view = core.chrome_view();
    assert_eq!(view.setup.pairing, "Paired");
    assert_eq!(view.setup.state, setup::SetupState::ExtensionAdded);
    assert_eq!(view.setup.profiles, Vec::<String>::new());
    let mut events = shared.events.subscribe();
    std::fs::create_dir_all(
        data.join("Profile 1/Extensions")
            .join(&chrome.config.extension_id),
    )
    .unwrap();
    // The owner's regular chrome_status poll can repeat exactly the same pairing answer.
    owner::update_pairing(&Arc::downgrade(shared), &reply);
    let event = events
        .try_recv()
        .expect("a profile appeared without a pairing change");
    assert!(
        matches!(event, crate::CoreEvent::ChromeControl { view } if view.setup.profiles == ["Profile 1"])
    );
    core.shutdown().await;
}
