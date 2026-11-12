//! Owner-key recovery and browser-local removal/reconnect, using an isolated memory device store and scripted owner.
use super::*;
use std::sync::{Arc, atomic::Ordering};

pub(super) async fn memory_keys(root: &std::path::Path) -> Arc<vw_keys::KeyService> {
    vw_keys::KeyService::start(vw_keys::KeyServiceConfig {
        managed_vault: None,
        vault: vw_vault::VaultConfig {
            location: vw_vault::location::VaultLocation::explicit(root.join("vault")),
            device_store: Arc::new(vw_vault::remember::MemoryDeviceStore::default()),
            kdf: vw_vault::KdfParams::default(),
            device_name: "test".into(),
        },
        proxy: vw_keyproxy::ProxyConfig::default(),
        test_api_base: "http://127.0.0.1:1".into(),
        settings_path: root.join("keys.json"),
        existing: vw_keys::ExistingKeys {
            dotenv: root.join(".env"),
            keyring: Arc::new(vw_keys::existing::MemoryVibeKeyring::default()),
        },
    })
    .await
    .unwrap()
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn removing_the_only_selected_browser_deletes_its_owner_and_setup() {
    for edge in [false, true] {
        let name = format!("single-browser-owner-{edge}");
        let root = std::env::var_os("VW_TEST_ROOT")
            .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
            .join(format!("{name}-{}", std::process::id()));
        let keys = memory_keys(&root).await;
        keys.create_chrome_owner().await.unwrap();
        let registration = registry_native_tests::TestRoot::new(&name);
        let core = crate::offline_core::start_with(&name, |config, dir| {
            let package = dir.join("package");
            std::fs::create_dir_all(package.join("bin")).unwrap();
            std::fs::write(package.join("bin").join(setup::helper_name()), b"helper").unwrap();
            config.keys = Some(keys.clone());
            config.chrome_control.package = Some(package);
            config.chrome_control.user_home = Some(dir.join("home"));
            config.chrome_control.macos = Some(false);
            config.chrome_control.windows_registry_root = format!(r"{}\Chrome", registration.key());
            config.chrome_control.edge_registry_root = format!(r"{}\Edge", registration.key());
        })
        .await;
        let chrome = &core.shared.chrome_control;
        let state = chrome.dir().unwrap();
        setup::install_for(
            &chrome.config,
            state,
            &core.shared.instructions.own_home,
            if edge {
                "Microsoft Edge"
            } else {
                "Google Chrome"
            },
        )
        .unwrap();
        let files = setup::receipt_files(&chrome.config, state).unwrap();
        chrome.owner.answering(|name, _| match name {
            "owner.browserUnpair" => json!({"unpaired":true}),
            _ => json!({"applied":true}),
        });
        core.chrome_control(
            crate::Origin::Ui,
            if edge {
                ChromeControlRequest::RemoveEdge {}
            } else {
                ChromeControlRequest::Remove {}
            },
        )
        .await
        .unwrap();
        assert!(
            keys.chrome_owner().await.unwrap().is_none(),
            "only one browser was selected"
        );
        assert!(!chrome.owner.is_running());
        assert!(!state.exists(), "remaining state: {}", state.display());
        for file in files {
            assert!(!file.exists(), "remaining file: {}", file.display());
        }
        core.shutdown().await;
        keys.shutdown().await;
    }
}

#[tokio::test]
async fn deleted_owner_credential_names_remove_then_connect_for_both_cards() {
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("browser-owner-credential-{}", std::process::id()));
    let keys = memory_keys(&root).await;
    keys.create_chrome_owner().await.unwrap();
    let core = crate::offline_core::start_with("browser-missing-owner", |config, dir| {
        config.keys = Some(keys.clone());
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    test_support::answering(&core.shared, |_, _| json!({}));
    let chrome = &core.shared.chrome_control;
    chrome.owner.stop().await;
    let state = chrome.dir().unwrap();
    std::fs::write(
        state.join("setup.json"),
        br#"{"files":[],"browsers":["Google Chrome","Microsoft Edge"],"selectedBrowsers":["Google Chrome","Microsoft Edge"],"userData":[]}"#,
    )
    .unwrap();
    // Delete outside the core's Remove workflow: its old registration and pairing still exist.
    keys.delete_chrome_owner().await.unwrap();
    assert!(owner::start(&core.shared).await.is_err());
    let view = core.chrome_view();
    for (setup, button) in std::iter::once((&view.setup, "Connect Chrome"))
        .chain(view.edge.as_deref().map(|edge| (edge, "Connect Edge")))
    {
        let addon = setup.addon.as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(addon.state).unwrap(),
            "ownerCredentialMissing"
        );
        assert_eq!(serde_json::to_value(addon.action).unwrap(), "remove");
        assert!(addon.instruction.contains("Remove…"));
        assert!(addon.instruction.contains(button));
    }
    if let Some(edge) = &view.edge {
        let reason =
            availability::reason(edge.addon.as_ref().unwrap(), agent_browser::Browser::Edge);
        assert!(
            reason.contains(&format!("{} card", vw_vocabulary::texts::EDGE_NAME)),
            "{reason}"
        );
        assert!(
            !reason.contains(vw_vocabulary::texts::CHROME_NAME),
            "{reason}"
        );
    }
    assert!(
        core.chrome_control(
            crate::Origin::Ui,
            commands::ChromeControlRequest::Connect {}
        )
        .await
        .is_err()
    );
    core.chrome_control(crate::Origin::Ui, commands::ChromeControlRequest::Remove {})
        .await
        .unwrap();
    assert_eq!(
        core.chrome_view().setup.addon.unwrap().state,
        AddonState::NotSetUp
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.join("setup.json")).unwrap()).unwrap();
    assert_eq!(receipt["browsers"], json!(["Microsoft Edge"]));
    assert!(
        chrome.owner.credential_missing.load(Ordering::SeqCst),
        "the remaining browser still needs its old registration removed"
    );
    #[cfg(feature = "your-edge")]
    {
        assert_eq!(
            core.chrome_view().edge.unwrap().addon.unwrap().state,
            AddonState::OwnerCredentialMissing
        );
        core.chrome_control(
            crate::Origin::Ui,
            commands::ChromeControlRequest::RemoveEdge {},
        )
        .await
        .unwrap();
        assert!(!state.exists(), "last removal cleans the shared setup");
        let view = core.chrome_view();
        assert_eq!(view.setup.addon.unwrap().state, AddonState::NotSetUp);
        assert_eq!(
            view.edge.unwrap().addon.unwrap().state,
            AddonState::NotSetUp
        );
        assert!(!chrome.owner.credential_missing.load(Ordering::SeqCst));
    }
    assert!(
        keys.chrome_owner().await.unwrap().is_none(),
        "reading a missing credential must never mint one"
    );
    core.shutdown().await;
    keys.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn browser_removal_preserves_the_owner_key_until_the_last_registration() {
    use crate::Origin;
    for edge_first in [false, true] {
        let name = format!("browser-owner-last-{edge_first}");
        let root = std::env::var_os("VW_TEST_ROOT")
            .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
            .join(format!("{name}-{}", std::process::id()));
        let keys = memory_keys(&root).await;
        keys.create_chrome_owner().await.unwrap();
        let original = keys.chrome_owner().await.unwrap().unwrap();
        let registration = registry_native_tests::TestRoot::new(&name);
        let core = crate::offline_core::start_with(&name, |config, dir| {
            let package = dir.join("package");
            std::fs::create_dir_all(package.join("bin")).unwrap();
            std::fs::write(
                package.join("bin").join(setup::helper_name()),
                b"shared helper",
            )
            .unwrap();
            config.keys = Some(keys.clone());
            config.chrome_control.package = Some(package);
            config.chrome_control.user_home = Some(dir.join("home"));
            config.chrome_control.windows_registry_root =
                format!(r"{}\NativeMessagingHosts", registration.key());
            config.chrome_control.edge_registry_root =
                format!(r"{}\Edge\NativeMessagingHosts", registration.key());
        })
        .await;
        let chrome = &core.shared.chrome_control;
        let state = chrome.dir().unwrap();
        let helper = state.join("bin").join(setup::helper_name());
        setup::install(&chrome.config, state, &core.shared.instructions.own_home).unwrap();
        setup::register_edge(&chrome.config, state).unwrap();
        chrome.owner.answering(|name, _| {
            if name == "owner.browserUnpair" {
                json!({"unpaired":true})
            } else {
                json!({"applied":true})
            }
        });
        let remove = || {
            if edge_first {
                ChromeControlRequest::RemoveEdge {}
            } else {
                ChromeControlRequest::Remove {}
            }
        };
        core.chrome_control(Origin::Ui, remove()).await.unwrap();
        // A poll already in flight may still name both browsers and their shared fingerprint.
        owner::update_pairing(
            &Arc::downgrade(&core.shared),
            &json!({"structuredContent":{
                "profiles":[{"profileId":"chrome:one","connected":true},
                            {"profileId":"edge:one","connected":true}],
                "ownerFingerprint":"0123456789abcdef"
            }}),
        );
        let (removed, remaining) = if edge_first {
            (&chrome.owner.edge_pairing, &chrome.owner.pairing)
        } else {
            (&chrome.owner.pairing, &chrome.owner.edge_pairing)
        };
        assert_eq!(*removed.lock().unwrap(), (PairingState::Unknown, None));
        assert_eq!(
            *remaining.lock().unwrap(),
            (PairingState::Paired, Some("0123456789abcdef".into()))
        );
        assert!(
            keys.chrome_owner()
                .await
                .unwrap()
                .is_some_and(|key| key.expose() == original.expose()),
            "the other browser retains exactly its existing owner key"
        );
        assert!(helper.is_file());
        assert!(chrome.owner.is_running());
        let last = if edge_first {
            ChromeControlRequest::Remove {}
        } else {
            ChromeControlRequest::RemoveEdge {}
        };
        core.chrome_control(Origin::Ui, last).await.unwrap();
        assert!(keys.chrome_owner().await.unwrap().is_none());
        assert!(!state.exists());
        assert!(!chrome.owner.is_running());
        core.shutdown().await;
        keys.shutdown().await;
    }
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn reconnecting_removed_chrome_keeps_edges_owner_key_and_registration() {
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("browser-owner-reconnect-{}", std::process::id()));
    let keys = memory_keys(&root).await;
    keys.create_chrome_owner().await.unwrap();
    let original = keys.chrome_owner().await.unwrap().unwrap();
    let registration = registry_native_tests::TestRoot::new("browser-reconnect-owner");
    let core = crate::offline_core::start_with("browser-reconnect-owner", |config, dir| {
        let package = dir.join("package");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(
            package.join("bin").join(setup::helper_name()),
            b"shared helper",
        )
        .unwrap();
        config.keys = Some(keys.clone());
        config.chrome_control.package = Some(package);
        config.chrome_control.user_home = Some(dir.join("home"));
        config.chrome_control.macos = Some(false);
        config.chrome_control.launcher = Some(Arc::new(|_| Ok(())));
        config.chrome_control.windows_registry_root =
            format!(r"{}\NativeMessagingHosts", registration.key());
        config.chrome_control.edge_registry_root =
            format!(r"{}\Edge\NativeMessagingHosts", registration.key());
    })
    .await;
    let chrome = &core.shared.chrome_control;
    let state = chrome.dir().unwrap();
    setup::install(&chrome.config, state, &core.shared.instructions.own_home).unwrap();
    setup::register_edge(&chrome.config, state).unwrap();
    chrome.owner.answering(|name, _| match name {
        "owner.browserUnpair" => json!({"unpaired":true}),
        "owner.enroll.mint" => json!({"none":"no_waiting"}),
        _ => json!({"applied":true}),
    });
    core.chrome_control(crate::Origin::Ui, ChromeControlRequest::Remove {})
        .await
        .unwrap();
    let crate::CommandResult::ChromeControl {
        offer: Some(offer), ..
    } = core
        .chrome_control(crate::Origin::Ui, ChromeControlRequest::OfferSetup {})
        .await
        .unwrap()
    else {
        panic!("setup offer")
    };
    core.chrome_control(
        crate::Origin::Ui,
        ChromeControlRequest::Setup {
            offer_id: offer.offer_id,
        },
    )
    .await
    .unwrap();
    assert!(setup::registered(
        &chrome.config,
        Some(state),
        "Google Chrome"
    ));
    assert!(setup::registered(
        &chrome.config,
        Some(state),
        "Microsoft Edge"
    ));
    assert!(
        keys.chrome_owner()
            .await
            .unwrap()
            .is_some_and(|key| key.expose() == original.expose())
    );
    core.shutdown().await;
    keys.shutdown().await;
}
