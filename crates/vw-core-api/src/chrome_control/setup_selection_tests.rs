//! Upgrade selection tests use old receipts and isolated files/registry roots; no real browser or keyring.
use super::*;

fn legacy(s: &Scratch, chrome: bool, edge: bool) -> Receipt {
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    install_for(&s.config(), &s.state(), &s.vibe_home(), "Microsoft Edge").unwrap();
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.state().join(RECEIPT)).unwrap()).unwrap();
    saved.as_object_mut().unwrap().remove("selectedBrowsers");
    std::fs::write(s.state().join(RECEIPT), saved.to_string()).unwrap();
    super::super::super::store::save_paired_once(&s.state(), chrome).unwrap();
    super::super::super::store::save_paired_once(&s.state().join("edge"), edge).unwrap();
    read_receipt(&s.config(), &s.state()).unwrap()
}

fn assert_registered(s: &Scratch, chrome: bool, edge: bool) {
    let config = s.config();
    let receipt = read_receipt(&config, &s.state()).unwrap();
    for (browser, present) in [("Google Chrome", chrome), ("Microsoft Edge", edge)] {
        assert_eq!(receipt.browsers.iter().any(|name| name == browser), present);
        let target = targets(&s.root.join("home"), &config)
            .into_iter()
            .find(|target| target.name == browser)
            .unwrap();
        if let Some(hosts) = target.hosts {
            assert_eq!(hosts.join(format!("{HOST_NAME}.json")).exists(), present);
        }
        #[cfg(windows)]
        if let Some(root) = target.registry {
            assert_eq!(
                registration(&format!(r"{root}\{HOST_NAME}")).0,
                Some(i32::from(!present))
            );
        }
    }
}

#[test]
fn legacy_refresh_keeps_only_paired_browsers_and_last_remove_cleans_everything() {
    for (chrome, edge) in [(true, false), (false, true), (true, true), (false, false)] {
        let s = Scratch::new(&format!("selection-refresh-{chrome}-{edge}"));
        let before = legacy(&s, chrome, edge);
        let history = [
            s.state().join("paired.json"),
            s.state().join("edge/paired.json"),
        ];
        let bytes = history.each_ref().map(|path| std::fs::read(path).unwrap());
        super::super::super::helper_refresh::refresh(&s.config(), &s.state()).unwrap();
        assert_registered(&s, chrome, edge);
        assert_eq!(
            history.each_ref().map(|path| std::fs::read(path).unwrap()),
            bytes
        );
        assert_eq!(has_other(&s.config(), &s.state(), "Google Chrome"), edge);
        if chrome && edge {
            scoped_removal::remove_browser(&s.config(), &s.state(), "Google Chrome", true).unwrap();
            assert_registered(&s, false, true);
            assert!(s.state().join("bin").join(helper_name()).is_file());
        }
        scoped_removal::remove_browser(
            &s.config(),
            &s.state(),
            if edge {
                "Microsoft Edge"
            } else {
                "Google Chrome"
            },
            false,
        )
        .unwrap();
        assert!(!s.state().exists());
        for file in before.files {
            assert!(!file.exists(), "{}", file.display());
        }
        #[cfg(windows)]
        for key in before.registry_keys {
            assert_eq!(registration(&key).0, Some(1));
        }
    }
}

#[test]
fn legacy_receipt_never_counts_unpaired_browser_even_before_refresh() {
    let s = Scratch::new("selection-remove-first");
    legacy(&s, true, false);
    assert!(!has_other(&s.config(), &s.state(), "Google Chrome"));
    scoped_removal::remove_browser(&s.config(), &s.state(), "Google Chrome", false).unwrap();
    assert!(!s.state().exists());
}

#[test]
fn legacy_repair_does_not_recreate_missing_unselected_registration() {
    let s = Scratch::new("selection-missing");
    let before = legacy(&s, true, false);
    for file in before
        .files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
    {
        std::fs::remove_file(file).unwrap();
    }
    for key in before.registry_keys {
        registry_delete(&key, &manifest_path(&s.state(), "Google Chrome")).unwrap();
    }
    repair_registrations(&s.config(), &s.state(), Some("Microsoft Edge")).unwrap();
    repair_registrations(&s.config(), &s.state(), None).unwrap();
    assert_registered(&s, true, false);
}

#[test]
fn new_selection_survives_refresh_without_pairing_and_receipt_is_unchanged() {
    let s = Scratch::new("selection-new");
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    install_for(&s.config(), &s.state(), &s.vibe_home(), "Microsoft Edge").unwrap();
    let before = std::fs::read(s.state().join(RECEIPT)).unwrap();
    super::super::super::helper_refresh::refresh(&s.config(), &s.state()).unwrap();
    assert_registered(&s, true, true);
    assert_eq!(std::fs::read(s.state().join(RECEIPT)).unwrap(), before);
}

#[test]
fn selecting_the_old_unpaired_browser_registers_it_again_without_erasing_other_pairing() {
    let s = Scratch::new("selection-reconnect");
    legacy(&s, true, false);
    install_for(&s.config(), &s.state(), &s.vibe_home(), "Microsoft Edge").unwrap();
    super::super::super::helper_refresh::refresh(&s.config(), &s.state()).unwrap();
    assert_registered(&s, true, true);
    assert_eq!(
        std::fs::read(s.state().join("paired.json")).unwrap(),
        b"true"
    );
}

#[test]
fn unreadable_or_corrupt_pairing_history_never_selects_a_browser() {
    for directory in [false, true] {
        let s = Scratch::new(&format!("selection-unreadable-{directory}"));
        legacy(&s, true, false);
        let history = s.state().join("edge/paired.json");
        std::fs::remove_file(&history).unwrap();
        if directory {
            std::fs::create_dir(&history).unwrap();
        } else {
            std::fs::write(&history, b"not json").unwrap();
        }
        repair_registrations(&s.config(), &s.state(), None).unwrap();
        assert_registered(&s, true, false);
        assert_eq!(
            std::fs::read(s.state().join("paired.json")).unwrap(),
            b"true"
        );
        assert!(history.exists(), "migration never deletes pairing evidence");
    }
}

#[cfg(unix)]
#[test]
fn interrupted_cleanup_cannot_promote_the_remaining_unselected_browser() {
    let s = Scratch::new("selection-retry");
    legacy(&s, false, false);
    let edge = targets(&s.root.join("home"), &s.config())
        .remove(1)
        .hosts
        .unwrap()
        .join(format!("{HOST_NAME}.json"));
    let original = std::fs::read(&edge).unwrap();
    std::fs::write(&edge, b"foreign").unwrap();
    assert!(repair_registrations(&s.config(), &s.state(), None).is_err());
    assert!(!has_other(&s.config(), &s.state(), "Google Chrome"));
    assert_eq!(std::fs::read(&edge).unwrap(), b"foreign");
    std::fs::write(&edge, original).unwrap();
    repair_registrations(&s.config(), &s.state(), None).unwrap();
    assert_registered(&s, false, false);
}

#[test]
fn profile_history_is_browser_scoped_and_never_changed_by_migration() {
    let raw = "0b5f6a1e-27c4-4c1e-9d3a-6f1e2b3c4d5e";
    for (prefix, chrome, edge) in [
        ("", true, false),
        ("chrome:", true, false),
        ("edge:", false, true),
        ("safari:", false, false),
    ] {
        let s = Scratch::new(&format!(
            "selection-profile-{}",
            prefix.trim_end_matches(':')
        ));
        legacy(&s, false, false);
        let record = serde_json::json!({"type":"signed", "signature":"ab".repeat(64),
            "payload":serde_json::json!({"message":{"kind":"paired-profiles/1", "profiles":{
                format!("{prefix}{raw}"):format!("04{}", "ab".repeat(64))
            }}}).to_string()})
        .to_string();
        let path = s.state().join("paired-profiles.json");
        std::fs::write(&path, &record).unwrap();
        repair_registrations(&s.config(), &s.state(), None).unwrap();
        assert_registered(&s, chrome, edge);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), record);
    }
}

#[test]
fn migration_removes_edge_even_when_policy_excludes_its_setup_target() {
    let s = Scratch::new("selection-policy");
    legacy(&s, true, false);
    let mut config = s.config();
    config.edge_policy = Some(serde_json::json!({"NativeMessagingBlocklist":["*"]}));
    repair_registrations(&config, &s.state(), None).unwrap();
    assert_registered(&s, true, false);
}

#[test]
fn failed_selection_write_never_restores_missing_registrations() {
    let s = Scratch::new("selection-write-refused");
    let before = legacy(&s, true, false);
    for path in before
        .files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
    {
        std::fs::remove_file(path).unwrap();
    }
    let mut config = s.config();
    let state = s.state();
    config.file_access = Some(Arc::new(move |op, path| {
        if op == files::Access::CreateDir && path == state {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        } else {
            Ok(())
        }
    }));
    assert!(repair_registrations(&config, &s.state(), None).is_err());
    for path in before
        .files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
    {
        assert!(!path.exists());
    }
}

#[cfg(unix)]
#[test]
fn a_linked_pairing_record_is_not_selection_evidence() {
    let s = Scratch::new("selection-linked-history");
    legacy(&s, true, false);
    let path = s.state().join("edge/paired.json");
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(s.state().join("paired.json"), &path).unwrap();
    repair_registrations(&s.config(), &s.state(), None).unwrap();
    assert_registered(&s, true, false);
    assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn unambiguous_single_browser_and_old_macos_receipts_remain_unchanged() {
    for macos in [false, true] {
        let s = Scratch::new(&format!("selection-unambiguous-{macos}"));
        if macos {
            legacy(&s, false, false);
        } else {
            install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
            let mut saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(s.state().join(RECEIPT)).unwrap()).unwrap();
            saved.as_object_mut().unwrap().remove("selectedBrowsers");
            std::fs::write(s.state().join(RECEIPT), saved.to_string()).unwrap();
        }
        let before = std::fs::read(s.state().join(RECEIPT)).unwrap();
        let mut config = s.config();
        config.macos = Some(macos);
        migrate_selections(&config, &s.state()).unwrap();
        assert_eq!(std::fs::read(s.state().join(RECEIPT)).unwrap(), before);
    }
}

#[test]
fn chromium_migration_does_not_change_another_browsers_legacy_ownership() {
    let s = Scratch::new("selection-other-browser");
    legacy(&s, true, false);
    let path = s.state().join(RECEIPT);
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    receipt["browsers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("Mozilla Firefox"));
    receipt["userData"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(s.root.join("firefox-data")));
    std::fs::write(&path, receipt.to_string()).unwrap();
    migrate_selections(&s.config(), &s.state()).unwrap();
    let receipt = read_receipt(&s.config(), &s.state()).unwrap();
    assert_eq!(receipt.browsers, ["Google Chrome", "Mozilla Firefox"]);
    assert!(has_other(&s.config(), &s.state(), "Google Chrome"));
}
