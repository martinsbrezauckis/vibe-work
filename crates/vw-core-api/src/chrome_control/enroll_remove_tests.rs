//! Removal from one browser card preserves the other browser's owner, setup and pairing history.
//! Uses the offline core and scripted private owner; never touches a real browser or keyring.
use super::*;

#[tokio::test]
async fn removing_chrome_keeps_edge_setup_pairing_and_shared_owner() {
    let rig = Rig::new("remove-chrome-keep-edge", |name, _| {
        if name == "owner.browserUnpair" {
            json!({"unpaired":true})
        } else {
            json!({})
        }
    })
    .await;
    let chrome = &rig.core.shared.chrome_control;
    let state = chrome.dir().unwrap();
    let helper = state.join("bin").join(setup::helper_name());
    // Rig owns both disposable registry roots; the installer also records their manifests and selections.
    setup::install(
        &chrome.config,
        state,
        &rig.core.shared.instructions.own_home,
    )
    .unwrap();
    setup::register_edge(&chrome.config, state).unwrap();
    chrome.mark_paired();
    edge_state::remember_pairing(chrome, true);
    let host_name = format!("{}.json", setup::HOST_NAME);
    let manifests: Vec<_> = setup::receipt_files(&chrome.config, state)
        .unwrap()
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name == host_name.as_str())
        })
        .collect();
    assert!(
        !manifests.is_empty(),
        "both browsers need real native-host registration"
    );
    #[cfg(windows)]
    assert_native_hosts(chrome, true, true);
    *chrome.owner.edge_pairing.lock().unwrap() = (PairingState::Paired, Some(OURS.into()));
    let status = json!({"profiles":[{"profileId":"chrome:p","connected":true},{"profileId":"edge:p","connected":true}]});
    chrome.owner.connected_profiles.observe(&status);
    profiles::observe(chrome, &status);
    let edge_generation = chrome.browser_generation(agent_browser::Browser::Edge);
    rig.core
        .chrome_control(Origin::Ui, ChromeControlRequest::Remove {})
        .await
        .unwrap();
    assert!(
        helper.is_file(),
        "shared helper must survive while Edge is registered"
    );
    assert!(setup::registered(
        &chrome.config,
        Some(state),
        "Microsoft Edge"
    ));
    assert!(!setup::registered(
        &chrome.config,
        Some(state),
        "Google Chrome"
    ));
    assert_eq!(manifests.iter().filter(|path| path.is_file()).count(), 1);
    #[cfg(windows)]
    assert_native_hosts(chrome, false, true);
    assert!(
        chrome.owner.is_running(),
        "removing Chrome must keep Edge's owner pipe alive"
    );
    assert!(
        chrome
            .edge_paired_once
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    assert_eq!(
        chrome.owner.edge_pairing.lock().unwrap().0,
        PairingState::Paired
    );
    assert_eq!(
        chrome.browser_generation(agent_browser::Browser::Edge),
        edge_generation
    );
    assert_eq!(chrome.owner.connected_profiles.snapshot().len(), 1);
    assert_eq!(chrome.owner.connected_profiles.snapshot()[0].0, "edge:p");
    // An already dispatched status poll can finish after the owner acknowledged removal.
    owner::update_pairing(
        &Arc::downgrade(&rig.core.shared),
        &json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true},{"profileId":"edge:p","connected":true}]}}),
    );
    assert!(
        !chrome.paired_once.load(std::sync::atomic::Ordering::SeqCst),
        "an old status reply cannot restore a removed pairing"
    );
    assert_eq!(chrome.owner.connected_profiles.snapshot().len(), 1);
    // PREDICT: fails before the change. Removing Chrome left its identity in the inventory, so after Connect again
    // it came back as an Offline Chrome profile next to the new pairing. It is removed with the browser.
    assert_eq!(shown_profiles(&rig.core), ["edge:p"]);
    assert!(
        rig.calls
            .lock()
            .unwrap()
            .iter()
            .any(|(name, args)| name == "owner.browserUnpair"
                && args == &json!({"browser":"chrome"}))
    );
    rig.core.shutdown().await;
}

#[cfg(windows)]
fn assert_native_hosts(chrome: &ChromeControl, chrome_present: bool, edge_present: bool) {
    use extension_registry::{Key, Native, Registry};
    let manifest = chrome
        .dir()
        .unwrap()
        .join("host")
        .join(format!("{}.json", setup::HOST_NAME));
    for (root, present) in [
        (&chrome.config.windows_registry_root, chrome_present),
        (&chrome.config.edge_registry_root, edge_present),
    ] {
        let key = Native
            .open(&format!(r"{root}\{}", setup::HOST_NAME))
            .unwrap();
        assert_eq!(key.is_some(), present, "{root}");
        if let Some(key) = key {
            assert_eq!(
                key.host_value().unwrap(),
                Some(manifest.to_string_lossy().replace('/', "\\"))
            );
        }
    }
}

fn shown_profiles(core: &crate::Core) -> Vec<String> {
    profiles::view(&core.shared)
        .into_iter()
        .map(|row| row.profile_id)
        .collect()
}

#[tokio::test]
async fn removing_last_browser_cleans_shared_setup_after_owner_unpair() {
    let rig = Rig::new("remove-last-browser", |name, _| {
        if name == "owner.browserUnpair" {
            json!({"unpaired":true})
        } else {
            json!({})
        }
    })
    .await;
    let chrome = &rig.core.shared.chrome_control;
    profiles::observe(
        chrome,
        &json!({"profiles":[{"profileId":"chrome:p","connected":true}]}),
    );
    rig.core
        .chrome_control(Origin::Ui, ChromeControlRequest::Remove {})
        .await
        .unwrap();
    assert!(!chrome.dir().unwrap().exists());
    // PREDICT: fails before the change. The deleted folder took the saved inventory, but the records stayed in
    // memory and the next save wrote them back. Nothing of the removed inventory is kept.
    assert!(chrome.profiles.lock().unwrap().records.is_empty());
    assert!(!chrome.owner.is_running());
    assert!(
        rig.calls
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "owner.browserUnpair")
    );
    rig.core.shutdown().await;
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn connect_migrates_old_receipt_without_revoking_selected_pairing() {
    for edge in [false, true] {
        let rig = Rig::new(&format!("connect-upgrade-selection-{edge}"), |_, _| {
            json!({})
        })
        .await;
        let chrome = &rig.core.shared.chrome_control;
        let state = chrome.dir().unwrap();
        setup::install(
            &chrome.config,
            state,
            &rig.core.shared.instructions.own_home,
        )
        .unwrap();
        setup::register_edge(&chrome.config, state).unwrap();
        if edge {
            edge_state::remember_pairing(chrome, true);
        } else {
            chrome.mark_paired();
        }
        let path = state.join("setup.json");
        let mut receipt: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        receipt.as_object_mut().unwrap().remove("selectedBrowsers");
        std::fs::write(&path, receipt.to_string()).unwrap();
        rig.core
            .chrome_control(
                Origin::Ui,
                if edge {
                    ChromeControlRequest::OpenEdgeStore {}
                } else {
                    ChromeControlRequest::Connect {}
                },
            )
            .await
            .unwrap();
        let receipt: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            receipt["browsers"],
            json!([if edge {
                "Microsoft Edge"
            } else {
                "Google Chrome"
            }])
        );
        assert!(store::paired_once(&if edge {
            state.join("edge")
        } else {
            state.to_path_buf()
        }));
        assert!(
            !rig.calls
                .lock()
                .unwrap()
                .iter()
                .any(|(method, _)| method == "owner.browserUnpair")
        );
        rig.core.shutdown().await;
    }
}
