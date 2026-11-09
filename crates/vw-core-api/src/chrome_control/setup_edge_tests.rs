//! Browser setup isolation and shared receipt removal; uses disposable homes and the real installer.
use super::*;

#[test]
fn chrome_setup_omits_edge_registration_when_edge_policy_refuses() {
    let scratch = Scratch::new("edge-blocked-chrome");
    let mut config = scratch.config();
    config.edge_policy = Some(serde_json::json!({"NativeMessagingBlocklist":[HOST_NAME]}));
    let allowed = scratch.config();
    let edge = targets(&scratch.root.join("home"), &allowed).remove(1);
    assert_eq!(
        super::super::super::edge_policy::refusal(&config),
        Some(super::super::super::edge_policy::BLOCKED)
    );
    install(&config, &scratch.state(), &scratch.vibe_home()).unwrap();
    let receipt = read_receipt(&scratch.config(), &scratch.state()).unwrap();
    assert_eq!(receipt.browsers, ["Google Chrome"]);
    assert_eq!(receipt.user_data.len(), 1);
    if let Some(hosts) = edge.hosts {
        assert!(!hosts.join(format!("{HOST_NAME}.json")).exists());
    }
    #[cfg(windows)]
    assert_eq!(
        registration(&format!(r"{}\{HOST_NAME}", config.edge_registry_root)).0,
        Some(1)
    );
    config.edge_policy = None;
    let mixed_state = PathBuf::from(scratch.state().to_str().unwrap().replace('\\', "/"));
    register_edge(&config, &mixed_state).unwrap();
    #[cfg(windows)]
    {
        let installed = registration(&format!(r"{}\{HOST_NAME}", config.edge_registry_root));
        let manifest = manifest_path(&scratch.state(), "Microsoft Edge");
        assert_eq!(installed.0, Some(0));
        assert!(!manifest.to_str().unwrap().contains('/'));
        assert!(String::from_utf8_lossy(&installed.1).contains(manifest.to_str().unwrap()));
    }
    assert_eq!(
        read_receipt(&scratch.config(), &scratch.state())
            .unwrap()
            .browsers,
        ["Google Chrome", "Microsoft Edge"]
    );
    remove(&scratch.config(), &scratch.state()).unwrap();
}

#[test]
fn edge_registration_rechecks_policy_even_for_an_existing_receipt() {
    let scratch = Scratch::new("edge-blocked-upgrade");
    let mut config = scratch.config();
    install(&config, &scratch.state(), &scratch.vibe_home()).unwrap();
    register_edge(&config, &scratch.state()).unwrap();
    let before = std::fs::read(scratch.state().join(RECEIPT)).unwrap();
    config.edge_policy = Some(serde_json::json!({"NativeMessagingUserLevelHosts":false}));
    assert_eq!(
        register_edge(&config, &scratch.state()),
        Err(super::super::super::edge_policy::BLOCKED.into())
    );
    assert_eq!(
        std::fs::read(scratch.state().join(RECEIPT)).unwrap(),
        before
    );
    remove(&scratch.config(), &scratch.state()).unwrap();
}

#[test]
fn separate_browser_setups_keep_profile_labels_separate() {
    let scratch = Scratch::new("edge");
    let home = scratch.root.join("home");
    let state = scratch.state();
    let config = scratch.config();
    let places = targets(&home, &config);
    let expected_names = vec!["Google Chrome", "Microsoft Edge"];
    assert_eq!(
        places.iter().map(|t| t.name).collect::<Vec<_>>(),
        expected_names
    );
    // Native registration mutations run on Unix only; Windows uses disposable registry keys in its own test.
    if cfg!(windows) {
        return;
    }
    let edge = &places[1];
    let expected = if config.is_macos() {
        "Library/Application Support/Microsoft Edge"
    } else {
        ".config/microsoft-edge"
    };
    assert_eq!(edge.user_data, home.join(expected));
    assert_eq!(
        edge.hosts.as_ref().unwrap(),
        &home.join(expected).join("NativeMessagingHosts")
    );
    std::fs::create_dir_all(edge.user_data.join("Default/Extensions").join(EXTENSION_ID)).unwrap();
    let view = install(&config, &state, &scratch.vibe_home()).unwrap();
    assert_eq!(
        read_receipt(&config, &state).unwrap().browsers,
        ["Google Chrome"]
    );
    install_for(&config, &state, &scratch.vibe_home(), "Microsoft Edge").unwrap();
    assert!(
        view.profiles.is_empty(),
        "Edge must never appear among Chrome profiles"
    );
    let receipt = read_receipt(&config, &state).unwrap();
    assert_eq!(receipt.browsers, expected_names);
    for place in &places {
        let manifest = place
            .hosts
            .as_ref()
            .unwrap()
            .join(format!("{HOST_NAME}.json"));
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        if place.name == "Mozilla Firefox" {
            assert_eq!(
                value["allowed_extensions"],
                serde_json::json!([FIREFOX_EXTENSION_ID])
            );
        } else {
            assert_eq!(
                value["allowed_origins"],
                serde_json::json!([format!("chrome-extension://{EXTENSION_ID}/")])
            );
        }
        assert_eq!(
            value["path"],
            state.join("bin").join(helper_name()).to_str().unwrap()
        );
        assert!(receipt.files.contains(&manifest));
    }
    remove(&config, &state).unwrap();
    for path in receipt.files {
        assert!(!path.exists(), "{}", path.display());
    }
    assert!(
        edge.user_data
            .join("Default/Extensions")
            .join(EXTENSION_ID)
            .is_dir()
    );
}

#[test]
fn edge_store_uses_its_own_launcher_and_the_existing_store_id() {
    let opened = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = opened.clone();
    let config = ChromeControlConfig {
        launcher: Some(Arc::new(|_| panic!("Edge setup launched Chrome"))),
        edge_launcher: Some(Arc::new(move |url| {
            captured.lock().unwrap().push(url.to_owned());
            Ok(())
        })),
        ..Default::default()
    };
    open_store_for(&config, true).unwrap();
    assert_eq!(*opened.lock().unwrap(), [store_url(EXTENSION_ID)]);
}

#[cfg(unix)]
#[test]
fn adding_edge_to_a_chrome_receipt_keeps_the_shared_helper_and_removes_both() {
    let scratch = Scratch::new("edge-upgrade");
    let config = scratch.config();
    let state = scratch.state();
    install(&config, &state, &scratch.vibe_home()).unwrap();
    let edge = targets(&scratch.root.join("home"), &config).remove(1);
    let manifest = edge.hosts.unwrap().join(format!("{HOST_NAME}.json"));
    assert!(!manifest.exists());
    assert_eq!(
        read_receipt(&config, &state).unwrap().browsers,
        ["Google Chrome"]
    );
    let helper = state.join("bin").join(helper_name());
    let before = std::fs::metadata(&helper).unwrap().modified().unwrap();
    register_edge(&config, &state).unwrap();
    register_edge(&config, &state).unwrap();
    assert_eq!(
        std::fs::metadata(&helper).unwrap().modified().unwrap(),
        before
    );
    assert_eq!(read_receipt(&config, &state).unwrap().browsers.len(), 2);
    remove(&config, &state).unwrap();
    assert!(!manifest.exists());
}
