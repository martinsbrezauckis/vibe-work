//! Per-user native-host registration in the registry: the default and configured keys, native path separators,
//! and rollback that leaves the person's real browser registration untouched.
#[cfg(windows)]
use super::tests::{Scratch, registration};
use super::*;

#[test]
fn the_registry_default_is_chromes_per_user_native_host_root() {
    assert_eq!(
        ChromeControlConfig::default().windows_registry_root,
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts"
    );
}

#[test]
fn windows_registry_manifest_paths_use_native_separators_for_both_browsers() {
    for root in [r"C:\Users\Test Person\data", "C:/Users/Test Person/data"] {
        let state = Path::new(root).join("chrome-control");
        for browser in ["Google Chrome", "Microsoft Edge"] {
            assert_eq!(
                windows_native_path(&manifest_path(&state, browser)),
                format!(r"C:\Users\Test Person\data\chrome-control\host\{HOST_NAME}.json"),
                "{browser}: {root}"
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn set_up_and_remove_use_the_configured_key_and_leave_real_chrome_untouched() {
    let s = Scratch::new("registry");
    let config = s.config();
    let real = format!(r"HKCU\Software\Google\Chrome\NativeMessagingHosts\{HOST_NAME}");
    let before = registration(&real);
    let real_edge = format!(r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\{HOST_NAME}");
    let edge_before = registration(&real_edge);
    let key = format!(r"{}\{HOST_NAME}", config.windows_registry_root);
    assert_eq!(
        targets(&s.root.join("home"), &config)[0].registry,
        Some(config.windows_registry_root.as_str())
    );
    assert_eq!(registration(&key).0, Some(1));
    // Configured roots can contain forward slashes; persisted native paths must not.
    let mixed_state = PathBuf::from(s.state().to_str().unwrap().replace('\\', "/"));
    let view = install(&config, &mixed_state, &s.vibe_home()).unwrap();
    #[cfg(feature = "your-edge")]
    register_edge(&config, &mixed_state).unwrap();
    assert_eq!(view.state, SetupState::WaitingForExtension);
    let installed = registration(&key);
    assert_eq!(installed.0, Some(0));
    let manifest = s.state().join("host").join(format!("{HOST_NAME}.json"));
    assert!(String::from_utf8_lossy(&installed.1).contains(manifest.to_str().unwrap()));
    assert!(!manifest.to_str().unwrap().contains('/'));
    let host: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(
        host["path"],
        s.state().join("bin").join(helper_name()).to_str().unwrap()
    );
    assert!(!host["path"].as_str().unwrap().contains('/'));
    assert_eq!(registration(&real), before);
    assert_eq!(registration(&real_edge), edge_before);
    let receipt = read_receipt(&s.config(), &s.state()).unwrap();
    let mut expected = vec![key.clone()];
    if cfg!(feature = "your-edge") {
        let edge_key = format!(r"{}\{HOST_NAME}", config.edge_registry_root);
        let edge_installed = registration(&edge_key);
        assert_eq!(edge_installed.0, Some(0));
        assert!(String::from_utf8_lossy(&edge_installed.1).contains(manifest.to_str().unwrap()));
        expected.push(edge_key);
    }

    assert_eq!(receipt.registry_keys, expected);
    assert!(!receipt.user_data[0].to_str().unwrap().contains('/'));
    remove(&s.config(), &s.state()).unwrap();
    for key in expected {
        assert_eq!(registration(&key).0, Some(1));
    }
    assert!(!s.state().exists());
    assert_eq!(registration(&real), before);
    assert_eq!(registration(&real_edge), edge_before);
}

#[cfg(windows)]
#[test]
fn a_failed_receipt_rolls_back_registration_and_scratch_drop_cleans_up_after_panic() {
    let real = format!(r"HKCU\Software\Google\Chrome\NativeMessagingHosts\{HOST_NAME}");
    let before = registration(&real);
    let result = std::panic::catch_unwind(|| {
        let s = Scratch::new("registry-unwind");
        let config = s.config();
        assert_eq!(
            targets(&s.root.join("home"), &config)[0].registry,
            Some(config.windows_registry_root.as_str())
        );
        let key = format!(r"{}\{HOST_NAME}", config.windows_registry_root);
        std::fs::create_dir_all(s.state().join("setup.json")).unwrap();
        assert!(install(&config, &s.state(), &s.vibe_home()).is_err());
        assert_eq!(registration(&key).0, Some(1));
        std::fs::remove_dir(s.state().join("setup.json")).unwrap();
        install(&config, &s.state(), &s.vibe_home()).unwrap();
        assert_eq!(registration(&key).0, Some(0));
        panic!("exercise scratch cleanup on test failure");
    });
    assert_eq!(
        result.unwrap_err().downcast_ref::<&str>(),
        Some(&"exercise scratch cleanup on test failure")
    );
    let registry_key = format!(
        r"HKCU\Software\VibeWorkTests\setup-registry-unwind-{}",
        std::process::id()
    );
    assert_eq!(registration(&registry_key).0, Some(1));
    assert_eq!(registration(&real), before);
}

/// Sets a test key's default value through reg.exe, as another setup would have left it.
#[cfg(windows)]
fn leave_value(key: &str, value: &str) {
    let status = std::process::Command::new("reg")
        .args(["add", key, "/ve", "/d", value, "/f"])
        .status()
        .unwrap();
    assert!(status.success());
}

/// Connect with a key another setup left: a value naming a missing manifest is replaced and said once; one naming
/// another setup's live manifest is refused with that path and the way out, and the key keeps its value.
///
/// PREDICT (before the fix): the stale case fails with "Browser helper registration belongs to another setup" on
/// every Connect.
#[cfg(windows)]
#[test]
fn connect_replaces_a_key_naming_a_missing_manifest_and_names_a_live_foreign_one() {
    let s = Scratch::new("registry-stale");
    let config = s.config();
    let key = format!(r"{}\{HOST_NAME}", config.windows_registry_root);
    let gone = s
        .root
        .join(r"old-lane\helper-install\native-host-manifest.json");
    let gone = gone.to_str().unwrap().to_owned();
    leave_value(&key, &gone);
    install(&config, &s.state(), &s.vibe_home()).unwrap();
    let manifest = s.state().join("host").join(format!("{HOST_NAME}.json"));
    assert!(String::from_utf8_lossy(&registration(&key).1).contains(manifest.to_str().unwrap()));
    let notice = config.setup_notice.take().expect("the replacement is said");
    assert!(notice.contains(&gone), "{notice}");
    assert!(config.setup_notice.take().is_none());
    remove(&config, &s.state()).unwrap();
    assert_eq!(registration(&key).0, Some(1));

    let other = s.root.join(r"another\host.json");
    let helper = s.root.join(r"another\chrome-control.exe");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(&helper, b"helper").unwrap();
    std::fs::write(
        &other,
        serde_json::to_vec(&serde_json::json!({"name": HOST_NAME, "path": helper})).unwrap(),
    )
    .unwrap();
    let other = other.to_str().unwrap().to_owned();
    leave_value(&key, &other);
    let error = install(&config, &s.state(), &s.vibe_home()).unwrap_err();
    assert!(
        error.starts_with(&format!(
            "Chrome is connected to another Vibe Work setup ({other})."
        )),
        "{error}"
    );
    assert!(error.ends_with("then retry Connect"), "{error}");
    assert!(String::from_utf8_lossy(&registration(&key).1).contains(&other));
    assert!(config.setup_notice.take().is_none());
}
