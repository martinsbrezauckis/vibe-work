//! Pairing before Chrome writes its extension directory, and later profile discovery on the status poll.
//! Uses an offline core and disposable package/home; never opens Chrome or a keyring.
use super::*;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn a_pairing_click_is_not_a_live_browser_connection() {
    let core = crate::offline_core::start("chrome-pairing-not-connected").await;
    owner::observe(
        &Arc::downgrade(&core.shared),
        &json!({"method":"notifications/owner/pairing", "params":{
            "state":"paired", "fingerprint":"0123456789abcdef"}}),
    )
    .unwrap();
    assert_ne!(
        core.shared.chrome_control.owner.pairing.lock().unwrap().0,
        "Paired"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn pairing_is_added_before_the_folder_and_later_profiles_emit_an_event() {
    profile_discovery(false).await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn edge_profile_discovery_emits_an_event_without_a_pairing_change() {
    profile_discovery(true).await;
}

async fn profile_discovery(edge: bool) {
    let name = if edge {
        "edge-pairing-discovery"
    } else {
        "chrome-pairing-discovery"
    };
    let core = crate::offline_core::start_with(name, |config, dir| {
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
        json!({"files":[],"browsers":[if edge { "Microsoft Edge" } else { "Google Chrome" }],"userData":[data]}).to_string(),
    )
    .unwrap();
    let reply = json!({"structuredContent":{"profiles":[{"profileId":if edge { "edge:one" } else { "chrome:one" },"connected":true}],"ownerFingerprint":"test"}});
    owner::update_pairing(&Arc::downgrade(shared), &reply);
    let view = core.chrome_view();
    let setup = if edge {
        view.edge.as_ref().unwrap()
    } else {
        &view.setup
    };
    assert_eq!(setup.pairing, "Paired");
    assert_eq!(setup.state, setup::SetupState::ExtensionAdded);
    assert_eq!(setup.profiles, Vec::<String>::new());
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
    let crate::CoreEvent::ChromeControl { view } = event else {
        panic!("browser view")
    };
    let setup = if edge {
        *view.edge.unwrap()
    } else {
        view.setup
    };
    assert_eq!(setup.profiles, ["Profile 1"]);
    core.shutdown().await;
}

#[tokio::test]
async fn connect_opens_the_extensions_pair_page_once_without_pairing_for_the_person() {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let launches = opened.clone();
    let core = crate::offline_core::start_with("chrome-connect-pair-page", move |config, _| {
        config.chrome_control.launcher = Some(Arc::new(move |url| {
            launches.lock().unwrap().push(url.to_owned());
            Ok(())
        }));
    })
    .await;
    let waiting = json!({"method":"notifications/owner/pairing", "params":{
        "profileId":"chrome:one", "state":"waiting", "fingerprint":"0123456789abcdef"}});
    // App startup alone never opens Chrome.
    owner::observe(&Arc::downgrade(&core.shared), &waiting).unwrap();
    assert!(opened.lock().unwrap().is_empty());
    core.shared
        .chrome_control
        .connect_requested
        .store(true, Ordering::SeqCst);
    owner::observe(&Arc::downgrade(&core.shared), &waiting).unwrap();
    owner::observe(&Arc::downgrade(&core.shared), &waiting).unwrap();
    assert_eq!(
        *opened.lock().unwrap(),
        [format!(
            "chrome-extension://{}/ui.html",
            setup::EXTENSION_ID
        )]
    );
    assert_ne!(
        core.shared.chrome_control.owner.pairing.lock().unwrap().0,
        "Paired"
    );
    core.shutdown().await;
}

#[cfg(any(feature = "your-edge", feature = "your-firefox"))]
#[tokio::test]
async fn other_browser_pairing_never_consumes_a_requested_chrome_connection() {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let launches = opened.clone();
    let core = crate::offline_core::start_with("separate-browser-connect", move |config, _| {
        config.chrome_control.launcher = Some(Arc::new(move |url| {
            launches.lock().unwrap().push(url.to_owned());
            Ok(())
        }));
    })
    .await;
    let chrome = &core.shared.chrome_control;
    chrome.connect_requested.store(true, Ordering::SeqCst);
    for profile in ["edge:one", "firefox:agent", "unknown"] {
        owner::observe(
            &Arc::downgrade(&core.shared),
            &json!({"method":"notifications/owner/pairing", "params":{
                "profileId":profile, "state":"waiting", "fingerprint":"0123456789abcdef"}}),
        )
        .unwrap();
        assert!(
            opened.lock().unwrap().is_empty(),
            "Another browser opened Chrome's pairing page"
        );
        assert!(chrome.connect_requested.load(Ordering::SeqCst));
    }
    core.shutdown().await;
}
#[tokio::test]
async fn remove_does_not_recreate_the_pairing_record() {
    let core = crate::offline_core::start_with("chrome-remove-paired-record", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        config.chrome_control.user_home = Some(dir.join("home"));
    })
    .await;
    let chrome = &core.shared.chrome_control;
    let dir = chrome.dir().unwrap().to_owned();
    chrome.mark_paired();
    assert!(dir.join("paired.json").is_file());
    core.chrome_control(crate::Origin::Ui, commands::ChromeControlRequest::Remove {})
        .await
        .unwrap();
    assert!(
        !dir.join("paired.json").exists(),
        "Remove recreated paired.json"
    );
    assert!(!dir.exists(), "empty app-owned folder remains");
    core.shutdown().await;
}

#[tokio::test]
async fn every_chrome_tool_names_not_set_up_and_offers_the_card() {
    let core = crate::offline_core::start_with("chrome-tool-setup-reason", |config, dir| {
        let package = dir.join("package");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(
            package.join("bin").join(setup::helper_name()),
            b"test helper",
        )
        .unwrap();
        config.chrome_control.package = Some(package);
        config.chrome_control.user_home = Some(dir.join("home"));
        config.chrome_control.launcher = Some(Arc::new(|_| Ok(())));
    })
    .await;
    let shared = &core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    for removed in [false, true] {
        if removed {
            shared.chrome_control.mark_paired();
            core.chrome_control(crate::Origin::Ui, commands::ChromeControlRequest::Remove {})
                .await
                .unwrap();
        }
        for (tool, args) in [
            ("chrome_status", json!({})),
            ("chrome_session", json!({"action":"open","profileId":"p"})),
            (
                "chrome_execute",
                json!({"sessionId":"old", "action":"page.read", "args":{}}),
            ),
            (
                "chrome_execute",
                json!({"sessionId":"old", "steps":[{"action":"page.read","args":{}}]}),
            ),
        ] {
            let result = gateway::call(shared, "s", tool, args).await;
            let text = match result {
                Ok(value) => value.to_string(),
                Err(text) => text,
            };
            assert!(
                text.contains("not set up")
                    && text.contains("addon_suggest")
                    && text.contains("Connect Chrome"),
                "{tool}, removed={removed}: {text}"
            );
            assert!(
                !text.contains(&format!(
                    "{} is not running",
                    vw_vocabulary::texts::CHROME_NAME
                )),
                "{text}"
            );
        }
    }
    core.shutdown().await;
}

#[tokio::test]
async fn external_removal_reports_restart_until_chrome_deletes_the_extension() {
    let core = crate::offline_core::start_with("chrome-remove-pending", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let chrome = &core.shared.chrome_control;
    let state = chrome.dir().unwrap();
    let data = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("chrome-data");
    let extension = data
        .join("Default/Extensions")
        .join(&chrome.config.extension_id);
    std::fs::create_dir_all(&extension).unwrap();
    let edge_data = chrome.config.user_home.as_ref().unwrap().join("edge-data");
    std::fs::create_dir_all(
        edge_data
            .join("Default/Extensions")
            .join(&chrome.config.extension_id),
    )
    .unwrap();
    std::fs::create_dir_all(state).unwrap();
    std::fs::write(
        state.join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome","Microsoft Edge"],"userData":[data,edge_data]}).to_string(),
    )
    .unwrap();
    // Chrome removes an extension itself only after a registration this app made is undone.
    super::extension_install::write_test_registration(&chrome.config, state);
    let result = core
        .chrome_control(crate::Origin::Ui, commands::ChromeControlRequest::Remove {})
        .await
        .unwrap();
    let crate::CommandResult::ChromeControl { view, .. } = result else {
        panic!("wrong result")
    };
    assert_eq!(view.setup.extension_restart, Some(true));
    assert_eq!(view.setup.extension_left, None);
    std::fs::remove_dir_all(extension).unwrap();
    assert_eq!(core.chrome_view().setup.extension_restart, None);
    core.shutdown().await;
}
