//! Missing owner credentials are recoverable setup facts, using an isolated memory device store and no real helper.
use super::*;
use std::sync::Arc;

#[tokio::test]
async fn deleted_owner_credential_names_remove_then_connect_for_both_cards() {
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("browser-owner-credential-{}", std::process::id()));
    let keys = vw_keys::KeyService::start(vw_keys::KeyServiceConfig {
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
    .unwrap();
    keys.create_chrome_owner().await.unwrap();
    let core = crate::offline_core::start_with("browser-missing-owner", |config, dir| {
        config.keys = Some(keys.clone());
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    test_support::answering(&core.shared, |_, _| json!({}));
    core.shared.chrome_control.owner.stop().await;
    let state = core.shared.chrome_control.dir().unwrap();
    std::fs::write(
        state.join("setup.json"),
        br#"{"files":[],"browsers":["Google Chrome","Microsoft Edge"],"userData":[]}"#,
    )
    .unwrap();
    // Delete outside the core's Remove workflow: its old registration and pairing still exist.
    keys.delete_chrome_owner().await.unwrap();
    assert!(owner::start(&core.shared).await.is_err());
    let view = core.chrome_view();
    for setup in std::iter::once(&view.setup).chain(view.edge.as_deref()) {
        let addon = setup.addon.as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(addon.state).unwrap(),
            "ownerCredentialMissing"
        );
        assert_eq!(serde_json::to_value(addon.action).unwrap(), "remove");
        assert!(addon.instruction.contains("Remove…"));
        let button = if addon.instruction.contains(vw_vocabulary::texts::EDGE_NAME) {
            "Set up…"
        } else {
            "Connect Chrome"
        };
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
    assert!(
        keys.chrome_owner().await.unwrap().is_none(),
        "reading a missing credential must never mint one"
    );
    core.shutdown().await;
    keys.shutdown().await;
}
