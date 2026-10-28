//! Set up is all or nothing, registers per user, and Remove deletes exactly what it made (scratch homes only).
use super::*;

struct Scratch {
    root: PathBuf,
    registry_key: String,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("vw-cc-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let package = root.join("package");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("bin").join(helper_name()), b"helper").unwrap();
        std::fs::write(
            package.join("plugin.json"),
            br#"{"name":"chrome-control","version":"0.1.0"}"#,
        )
        .unwrap();
        std::fs::write(
            package.join("mcp.json"),
            br#"{"mcpServers":{"chrome-control":{"type":"stdio","command":"./bin/chrome-control","args":["mcp"]}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("data/vibe-home")).unwrap();
        Self {
            root,
            registry_key: format!(
                r"HKCU\Software\VibeWorkTests\setup-{name}-{}",
                std::process::id()
            ),
        }
    }
    fn config(&self) -> ChromeControlConfig {
        ChromeControlConfig {
            package: Some(self.root.join("package")),
            user_home: Some(self.root.join("home")),
            windows_registry_root: format!(r"{}\NativeMessagingHosts", self.registry_key),
            ..ChromeControlConfig::default()
        }
    }
    fn state(&self) -> PathBuf {
        self.root.join("data").join("chrome-control")
    }
    fn vibe_home(&self) -> PathBuf {
        self.root.join("data/vibe-home")
    }
    fn every_file(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root.join("home"), &mut out);
        walk(&self.root.join("data"), &mut out);
        out.sort();
        out
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = registry_delete(&self.registry_key);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn the_registry_default_is_chromes_per_user_native_host_root() {
    assert_eq!(
        ChromeControlConfig::default().windows_registry_root,
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts"
    );
}

#[cfg(windows)]
fn registration(key: &str) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let output = std::process::Command::new("reg")
        .args(["query", key, "/s"])
        .output()
        .unwrap();
    assert!(matches!(output.status.code(), Some(0 | 1)));
    (output.status.code(), output.stdout, output.stderr)
}

#[cfg(windows)]
#[test]
fn set_up_and_remove_use_the_configured_key_and_leave_real_chrome_untouched() {
    let s = Scratch::new("registry");
    let config = s.config();
    let real = format!(r"HKCU\Software\Google\Chrome\NativeMessagingHosts\{HOST_NAME}");
    let before = registration(&real);
    let key = format!(r"{}\{HOST_NAME}", config.windows_registry_root);
    assert_eq!(
        targets(&s.root.join("home"), &config)[0].registry,
        Some(config.windows_registry_root.as_str())
    );
    assert_eq!(registration(&key).0, Some(1));
    let view = install(&config, &s.state(), &s.vibe_home()).unwrap();
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
    let receipt = read_receipt(&s.state()).unwrap();
    assert_eq!(receipt.registry_keys, vec![key.clone()]);
    assert!(!receipt.user_data[0].to_str().unwrap().contains('/'));
    remove(&s.state()).unwrap();
    assert_eq!(registration(&key).0, Some(1));
    assert!(!s.state().exists());
    assert_eq!(registration(&real), before);
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
        std::fs::create_dir_all(s.state().join("setup.new")).unwrap();
        assert!(install(&config, &s.state(), &s.vibe_home()).is_err());
        assert_eq!(registration(&key).0, Some(1));
        std::fs::remove_dir(s.state().join("setup.new")).unwrap();
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

#[cfg(not(windows))]
#[test]
fn set_up_registers_for_this_user_only_and_points_chrome_at_the_copied_helper() {
    let s = Scratch::new("ok");
    let view = install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    assert!(
        !s.vibe_home().join("plugins-off/chrome-control").exists(),
        "Set up must not install an agent launcher"
    );
    assert_eq!(view.state, SetupState::WaitingForExtension);
    assert_eq!(view.browsers, vec!["Google Chrome".to_owned()]);
    let hosts = if cfg!(target_os = "macos") {
        s.root
            .join("home/Library/Application Support/Google/Chrome/NativeMessagingHosts")
    } else {
        s.root
            .join("home/.config/google-chrome/NativeMessagingHosts")
    };
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(hosts.join(format!("{HOST_NAME}.json"))).unwrap())
            .unwrap();
    let helper = s.state().join("bin").join(helper_name());
    assert_eq!(manifest["path"], helper.to_string_lossy().as_ref());
    assert_eq!(
        manifest["allowed_origins"],
        serde_json::json!([format!("chrome-extension://{EXTENSION_ID}/")])
    );
    assert_eq!(std::fs::read(&helper).unwrap(), b"helper");
    assert!(!s.vibe_home().join("plugins/chrome-control").exists());
    assert!(!s.vibe_home().join("plugins-off/chrome-control").exists());
    // A Chrome profile that has the extension's folder: "extension added".
    std::fs::create_dir_all(
        s.root
            .join(if cfg!(target_os = "macos") {
                "home/Library/Application Support/Google/Chrome"
            } else {
                "home/.config/google-chrome"
            })
            .join("Default/Extensions")
            .join(EXTENSION_ID),
    )
    .unwrap();
    let later = super::view(&s.config(), Some(&s.state()), &s.vibe_home());
    assert_eq!(later.state, SetupState::ExtensionAdded);
    assert_eq!(later.profiles, vec!["Default".to_owned()]);
}

#[cfg(not(windows))]
#[test]
fn a_failed_host_registration_leaves_nothing_behind() {
    let s = Scratch::new("fail");
    // Chrome's host folder cannot be made: a file stands where its parent folder would be.
    let blocker = if cfg!(target_os = "macos") {
        s.root
            .join("home/Library/Application Support/Google/Chrome")
    } else {
        s.root.join("home/.config/google-chrome")
    };
    std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    std::fs::write(&blocker, b"not a folder").unwrap();
    let before = s.every_file();
    let error = install(&s.config(), &s.state(), &s.vibe_home()).unwrap_err();
    assert_ne!(error, "");
    assert_eq!(
        s.every_file(),
        before,
        "the helper copy and the plugin files are gone again"
    );
    assert!(!s.vibe_home().join("plugins-off/chrome-control").exists());
    assert_eq!(
        super::view(&s.config(), Some(&s.state()), &s.vibe_home()).state,
        SetupState::NotSetUp
    );
}

#[cfg(not(windows))]
#[test]
fn remove_deletes_exactly_what_set_up_made_and_the_saved_permissions() {
    let s = Scratch::new("remove");
    let before = s.every_file();
    std::fs::write(s.root.join("home/keep.txt"), b"the user's").unwrap();
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    std::fs::write(s.state().join("permissions.json"), b"{}").unwrap();
    remove(&s.state()).unwrap();
    let mut after = s.every_file();
    after.retain(|p| !p.ends_with("keep.txt"));
    assert_eq!(after, before);
    assert!(
        s.root.join("home/keep.txt").is_file(),
        "nothing else is touched"
    );
    assert!(!s.state().exists());
}

#[test]
fn set_up_refuses_without_a_package_or_its_native_helper() {
    let s = Scratch::new("package");
    let mut config = s.config();
    config.package = None;
    assert!(
        install(&config, &s.state(), &s.vibe_home())
            .unwrap_err()
            .contains(ABSENT)
    );
    std::fs::remove_file(s.root.join("package/bin").join(helper_name())).unwrap();
    assert!(
        install(&s.config(), &s.state(), &s.vibe_home())
            .unwrap_err()
            .contains("has no Chrome Control helper")
    );
    assert!(!super::view(&s.config(), Some(&s.state()), &s.vibe_home()).package);
    assert_eq!(s.every_file(), Vec::<PathBuf>::new());
}

#[test]
fn the_store_page_opens_through_the_launcher_with_the_configured_id() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let record = seen.clone();
    let config = ChromeControlConfig {
        extension_id: "abcdefghijklmnopabcdefghijklmnop".to_owned(),
        launcher: Some(Arc::new(move |url: &str| {
            record.lock().unwrap().push(url.to_owned());
            Ok(())
        })),
        ..ChromeControlConfig::default()
    };
    open_store(&config).unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            "https://chromewebstore.google.com/detail/abcdefghijklmnopabcdefghijklmnop".to_owned()
        ]
    );
}

#[cfg(not(windows))]
#[test]
fn readonly_session_packages_survive_chrome_remove_and_rollback() {
    let s = Scratch::new("readonly-boundary");
    let checkout = s.vibe_home().join("logs/session/plugins/packages/ab/hash");
    crate::readonly_fixture::readonly_package(&checkout);
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    remove(&s.state()).unwrap();
    assert!(checkout.join("skills/example/SKILL.md").is_file());
    let blocker = if cfg!(target_os = "macos") {
        s.root
            .join("home/Library/Application Support/Google/Chrome/NativeMessagingHosts")
    } else {
        s.root
            .join("home/.config/google-chrome/NativeMessagingHosts")
    };
    std::fs::remove_dir(&blocker).unwrap();
    std::fs::write(&blocker, b"blocked").unwrap();
    assert!(install(&s.config(), &s.state(), &s.vibe_home()).is_err());
    assert!(checkout.join("skills/example/SKILL.md").is_file());
    vw_platform::remove_owned_tree(s.root.parent().unwrap(), &s.root).unwrap();
}

#[test]
fn owned_native_package_needs_no_plugin_descriptor_or_mcp_launcher() {
    let s = Scratch::new("owned-package");
    std::fs::remove_file(s.root.join("package/plugin.json")).unwrap();
    std::fs::remove_file(s.root.join("package/mcp.json")).unwrap();
    assert!(
        packaged_helper(&s.config()).is_ok(),
        "the owned native package must not need plugin.json or mcp.json"
    );
}

#[test]
fn planted_helper_is_refused_before_copy_or_registration() {
    let s = Scratch::new("pin");
    let mut config = s.config();
    // SHA-256 of the fixture's original shipped bytes, "helper".
    config.helper_sha256 =
        Some("e81d3b0e9d82feaaf5f6e55bdff24731d7eee08632ffa63801e6397290c5d20a".to_owned());
    assert!(packaged_helper(&config).is_ok());
    std::fs::write(s.root.join("package/bin").join(helper_name()), b"planted").unwrap();
    assert!(!super::view(&config, Some(&s.state()), &s.vibe_home()).package);
    let error = install(&config, &s.state(), &s.vibe_home()).unwrap_err();
    assert_eq!(
        error,
        "The Chrome helper in this installation is not the one that shipped with Vibe Work. Reinstall Vibe Work."
    );
    assert_eq!(s.every_file(), Vec::<PathBuf>::new());
}
