//! macOS Settings and agent facts share reported profiles and permission failures; no protected browser probes.
//! Uses an offline core and the same permission fixture as native-host installation.
use super::*;
use crate::chrome_control::{self as chrome, PairingState, agent_browser::Browser};
use serde_json::json;

#[tokio::test]
async fn macos_live_profiles_and_versions_come_from_the_extension_and_disconnect_is_not_uninstalled()
 {
    let s = Scratch::new("mac-status");
    let (config, calls) = protected(&s);
    bundle(&s, "Google Chrome");
    let core =
        crate::offline_core::start_with("mac-live-facts", |c, _| c.chrome_control = config).await;
    let connection = &core.shared.chrome_control;
    install_for(
        &connection.config,
        connection.dir().unwrap(),
        &s.vibe_home(),
        "Google Chrome",
    )
    .unwrap();
    connection
        .owner
        .tools
        .lock()
        .unwrap()
        .push(json!({"name":"chrome_status"}));
    let view = chrome::commands::view(&core.shared);
    assert_eq!(
        view.setup.addon.unwrap().state,
        chrome::AddonState::NotConnected,
        "absence of a profile read does not prove an uninstalled extension"
    );
    connection
        .owner
        .connected_profiles
        .observe(&json!({"profiles":[
            {"profileId":"chrome:one","connected":true,"extensionVersion":"0.1.8"},
            {"profileId":"edge:other","connected":true,"extensionVersion":"99.0.0"}
        ]}));
    connection.owner.pairing.lock().unwrap().0 = PairingState::Paired;
    let view = chrome::commands::view(&core.shared);
    assert_eq!(view.setup.profiles, ["chrome:one"]);
    assert_eq!(view.setup.addon.as_ref().unwrap().label, "Paired");
    assert_eq!(
        view.setup.addon.unwrap().extension_version.as_deref(),
        Some("0.1.8")
    );
    assert_eq!(
        chrome::installation::version(&core.shared, Browser::Chrome).as_deref(),
        Some("0.1.8")
    );
    connection.owner.connected_profiles.clear();
    connection
        .paired_once
        .store(true, std::sync::atomic::Ordering::SeqCst);
    connection.owner.pairing.lock().unwrap().0 = PairingState::Disconnected;
    let view = chrome::commands::view(&core.shared);
    assert_eq!(
        view.setup.addon.unwrap().state,
        chrome::AddonState::ChromeClosed
    );
    assert_eq!(
        chrome::installation::version(&core.shared, Browser::Chrome),
        None
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(op, p)| *op == files::Access::ReadDir && p.ends_with("Google/Chrome"))
    );
    core.shutdown().await;
}

#[tokio::test]
async fn macos_permission_failure_survives_for_settings_and_agent_until_retry() {
    let s = Scratch::new("mac-permission-state");
    let (mut config, _) = protected(&s);
    bundle(&s, "Google Chrome");
    let blocked = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let gate = blocked.clone();
    let underlying = config.file_access.clone().unwrap();
    config.file_access = Some(Arc::new(move |op, path| {
        if gate.load(std::sync::atomic::Ordering::SeqCst) && path.ends_with("NativeMessagingHosts")
        {
            return Err(std::io::Error::from_raw_os_error(1));
        }
        underlying(op, path)
    }));
    let core =
        crate::offline_core::start_with("mac-permission-facts", |c, _| c.chrome_control = config)
            .await;
    let connection = &core.shared.chrome_control;
    let state = connection.dir().unwrap();
    let error =
        install_for(&connection.config, state, &s.vibe_home(), "Google Chrome").unwrap_err();
    assert!(error.contains("macOS blocked access"), "{error}");
    assert!(!error.contains("disk space"));
    for _ in 0..2 {
        let view = chrome::commands::view(&core.shared).setup.addon.unwrap();
        assert_eq!(serde_json::to_value(view.state).unwrap(), "accessBlocked");
        assert_eq!(view.label, "Blocked by macOS");
        assert!(view.instruction.contains("Files & Folders"));
        let agent = chrome::installation::addons(&core.shared);
        assert_eq!(agent[0]["stateCode"], "accessBlocked");
        assert_eq!(agent[0]["instruction"], view.instruction);
        assert_ne!(agent[1]["stateCode"], "accessBlocked");
    }
    let mut restarted = connection.config.clone();
    restarted.setup_problems = SetupProblems::default();
    let restored = view(&restarted, Some(state), &s.vibe_home());
    assert_eq!(
        serde_json::to_value(restored.addon.unwrap().state).unwrap(),
        "accessBlocked",
        "own-state receipt survives restart"
    );
    blocked.store(false, std::sync::atomic::Ordering::SeqCst);
    install_for(&connection.config, state, &s.vibe_home(), "Google Chrome").unwrap();
    assert_ne!(
        serde_json::to_value(
            chrome::commands::view(&core.shared)
                .setup
                .addon
                .unwrap()
                .state
        )
        .unwrap(),
        "accessBlocked"
    );
    core.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn macos_command_views_do_not_touch_either_browsers_registration() {
    let s = Scratch::new("mac-view-isolation");
    let (config, calls) = protected(&s);
    for browser in ["Google Chrome", "Microsoft Edge"] {
        bundle(&s, browser);
    }
    std::fs::create_dir_all(
        s.root
            .join("home/Library/Application Support/Microsoft Edge"),
    )
    .unwrap();
    let core = crate::offline_core::start_with("mac-view-isolation", |c, _| {
        c.chrome_control = config;
    })
    .await;
    let connection = &core.shared.chrome_control;
    for browser in ["Google Chrome", "Microsoft Edge"] {
        install_for(
            &connection.config,
            connection.dir().unwrap(),
            &s.vibe_home(),
            browser,
        )
        .unwrap();
    }
    calls.lock().unwrap().clear();
    // Connect's response and owner events render both cards. Rendering must not repair the other browser.
    for _ in 0..2 {
        let _ = chrome::commands::view(&core.shared);
        let _ = chrome::installation::addons(&core.shared);
    }
    let browser_calls: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, path)| {
            path.to_string_lossy()
                .contains("Library/Application Support")
        })
        .cloned()
        .collect();
    assert_eq!(browser_calls, []);
    core.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[test]
fn macos_remove_permission_failure_names_the_browser_and_keeps_agent_state() {
    let s = Scratch::new("mac-remove-diagnosis");
    let (mut config, _) = protected(&s);
    for browser in ["Google Chrome", "Microsoft Edge"] {
        bundle(&s, browser);
    }
    std::fs::create_dir_all(
        s.root
            .join("home/Library/Application Support/Microsoft Edge"),
    )
    .unwrap();
    for browser in ["Google Chrome", "Microsoft Edge"] {
        install_for(&config, &s.state(), &s.vibe_home(), browser).unwrap();
    }
    let underlying = config.file_access.clone().unwrap();
    config.file_access = Some(Arc::new(move |op, path| {
        if path.to_string_lossy().contains("Microsoft Edge") {
            return Err(std::io::Error::from_raw_os_error(1));
        }
        underlying(op, path)
    }));
    let reason = remove(&config, &s.state()).unwrap_err();
    assert!(reason.contains("macOS blocked access"), "{reason}");
    assert!(reason.contains("Microsoft Edge"), "{reason}");
    assert!(problem::blocked(
        &config,
        Some(&s.state()),
        "Microsoft Edge"
    ));
    assert!(!problem::blocked(
        &config,
        Some(&s.state()),
        "Google Chrome"
    ));
}

#[test]
fn macos_failed_registration_remains_visible_without_polling_the_host() {
    let s = Scratch::new("mac-registration-state");
    let (config, calls) = protected(&s);
    bundle(&s, "Google Chrome");
    install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
    let host = targets(&s.root.join("home"), &config)
        .remove(0)
        .hosts
        .unwrap()
        .join(format!("{HOST_NAME}.json"));
    std::fs::write(&host, br#"{"name":"foreign","path":"/foreign/helper"}"#).unwrap();
    assert!(repair_registrations(&config, &s.state(), Some("Google Chrome")).is_err());
    calls.lock().unwrap().clear();
    let mut restarted = config.clone();
    restarted.setup_problems = SetupProblems::default();
    assert_eq!(
        view(&restarted, Some(&s.state()), &s.vibe_home()).state,
        SetupState::RepairRequired
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, path)| path.starts_with(host.parent().unwrap()))
    );
}
