//! Explicit Connect with an existing receipt; real scratch manifests or a test-owned Windows registry key.
//! PREDICT: receipted Connect skips the prior-registration check, so stale and live foreign entries both get the
//! generic ownership refusal. Fresh setup and the Windows key's transaction guards are separate controls.
use super::*;

fn browsers() -> Vec<&'static str> {
    let mut browsers = vec!["Google Chrome"];
    if cfg!(feature = "your-edge") {
        browsers.push("Microsoft Edge");
    }
    browsers
}

fn config_for(s: &Scratch, macos: bool) -> ChromeControlConfig {
    let mut config = s.config();
    config.macos = Some(macos);
    #[cfg(unix)]
    if macos {
        use std::os::unix::fs::PermissionsExt;
        for browser in browsers() {
            let exe = s
                .root
                .join("home/Applications")
                .join(format!("{browser}.app/Contents/MacOS/{browser}"));
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"browser").unwrap();
            std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    if macos {
        for target in targets(&s.root.join("home"), &config) {
            std::fs::create_dir_all(target.user_data).unwrap();
        }
    }
    config
}

fn foreign(s: &Scratch, live: bool) -> (PathBuf, PathBuf) {
    let file = s.root.join("other/host.json");
    let helper = s.root.join("other").join("bin").join(helper_name());
    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    if live {
        std::fs::write(&helper, b"other helper").unwrap();
    }
    std::fs::write(&file, host_manifest(&helper, EXTENSION_ID)).unwrap();
    (file, helper)
}

fn leave_foreign(s: &Scratch, config: &ChromeControlConfig, browser: &str, file: &Path) -> PathBuf {
    let target = targets(&s.root.join("home"), config)
        .into_iter()
        .find(|target| target.name == browser)
        .unwrap();
    if let Some(hosts) = target.hosts {
        let path = hosts.join(format!("{HOST_NAME}.json"));
        std::fs::copy(file, &path).unwrap();
        return path;
    }
    #[cfg(windows)]
    {
        use crate::chrome_control::extension_registry::{Key, Native, Registry};
        let key = format!(r"{}\{HOST_NAME}", target.registry.unwrap());
        let status = std::process::Command::new("reg")
            .args(["add", &key, "/ve", "/d", file.to_str().unwrap(), "/f"])
            .status()
            .unwrap();
        assert!(status.success());
        let handle = Native.open(&key).unwrap().unwrap();
        assert!(!handle.marker_exists().unwrap());
        assert!(handle.lock().unwrap());
        assert!(handle.host_only().unwrap());
        assert_eq!(handle.host_value().unwrap().as_deref(), file.to_str());
        // Dropping this transaction leaves the independently created default value untouched.
        PathBuf::from(key)
    }
    #[cfg(not(windows))]
    panic!("a host folder is required off Windows");
}

fn registered_value(path: &Path) -> Vec<u8> {
    #[cfg(windows)]
    {
        crate::chrome_control::extension_registry::host_value(
            path.to_str().unwrap(),
            &crate::chrome_control::extension_registry::Native,
        )
        .unwrap()
        .unwrap_or_default()
        .into_bytes()
    }
    #[cfg(not(windows))]
    std::fs::read(path).unwrap()
}

fn shapes() -> Vec<bool> {
    if cfg!(windows) {
        vec![false]
    } else {
        vec![false, true]
    }
}

#[test]
fn reconnect_replaces_stale_registration_with_receipt() {
    for macos in shapes() {
        let s = Scratch::new(&format!("reconnect-stale-{macos}"));
        let config = config_for(&s, macos);
        for browser in browsers() {
            install_for(&config, &s.state(), &s.vibe_home(), browser).unwrap();
        }
        let (file, helper) = foreign(&s, false);
        for browser in browsers() {
            let path = leave_foreign(&s, &config, browser, &file);
            let before = registered_value(&path);
            // Passive status/refresh must keep refusing the stale foreign value until explicit Connect.
            assert!(repair_registrations(&config, &s.state(), Some(browser)).is_err());
            assert_eq!(registered_value(&path), before);
            install_for(&config, &s.state(), &s.vibe_home(), browser)
                .unwrap_or_else(|error| panic!("macos={macos}, {browser}: {error}"));
            assert_ne!(registered_value(&path), before);
            let notice = config.setup_notice.take().expect("replacement notice");
            assert!(notice.contains(&helper.display().to_string()), "{notice}");
            assert!(config.setup_notice.take().is_none());
            install_for(&config, &s.state(), &s.vibe_home(), browser).unwrap();
            assert!(
                config.setup_notice.take().is_none(),
                "ordinary reconnect is silent"
            );
        }
    }
}

#[test]
fn reconnect_names_live_foreign_registration_with_receipt() {
    for macos in shapes() {
        let s = Scratch::new(&format!("reconnect-live-{macos}"));
        let config = config_for(&s, macos);
        for browser in browsers() {
            install_for(&config, &s.state(), &s.vibe_home(), browser).unwrap();
        }
        let (file, helper) = foreign(&s, true);
        for browser in browsers() {
            let path = leave_foreign(&s, &config, browser, &file);
            let before = registered_value(&path);
            let error = install_for(&config, &s.state(), &s.vibe_home(), browser).unwrap_err();
            let shown = if cfg!(windows) { &file } else { &helper };
            assert!(error.contains(&shown.display().to_string()), "{error}");
            assert!(error.contains("Open that setup and use Remove"), "{error}");
            assert!(error.ends_with("then retry Connect"), "{error}");
            assert_eq!(registered_value(&path), before);
            assert!(config.setup_notice.take().is_none());
        }
    }
}

#[test]
fn simultaneous_reconnects_replace_stale_registration_once() {
    let s = Scratch::new("reconnect-concurrent");
    let config = config_for(&s, false);
    install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
    let (file, _) = foreign(&s, false);
    let path = leave_foreign(&s, &config, "Google Chrome", &file);
    let before = registered_value(&path);
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let connect = || {
            barrier.wait();
            install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
        };
        let first = scope.spawn(connect);
        let second = scope.spawn(connect);
        first.join().unwrap();
        second.join().unwrap();
    });
    assert_ne!(registered_value(&path), before);
    assert!(config.setup_notice.take().is_some());
    assert!(config.setup_notice.take().is_none());
}

#[cfg(windows)]
#[test]
fn reconnect_replaces_real_registry_values_naming_missing_manifests() {
    for missing_parent in [false, true] {
        let s = Scratch::new(&format!("reconnect-registry-missing-{missing_parent}"));
        let config = config_for(&s, false);
        for browser in browsers() {
            install_for(&config, &s.state(), &s.vibe_home(), browser).unwrap();
        }
        let (file, _) = foreign(&s, false);
        for browser in browsers() {
            let key = leave_foreign(&s, &config, browser, &file);
            if file.exists() {
                std::fs::remove_file(&file).unwrap();
                if missing_parent {
                    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
                }
            }
            install_for(&config, &s.state(), &s.vibe_home(), browser).unwrap();
            let own = windows_native_path(&manifest_path(&s.state(), browser));
            assert_eq!(registered_value(&key), own.as_bytes());
            let notice = config.setup_notice.take().unwrap();
            assert!(notice.contains(&file.display().to_string()), "{notice}");
            assert!(notice.contains("file that no longer exists"), "{notice}");
            assert!(config.setup_notice.take().is_none());
        }
    }
}

#[cfg(windows)]
#[test]
fn reconnect_preserves_real_registry_values_and_children_beside_a_stale_default() {
    for extra in ["value", "child", "marker"] {
        let s = Scratch::new(&format!("reconnect-registry-extra-{extra}"));
        let config = config_for(&s, false);
        install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
        let (file, _) = foreign(&s, false);
        let key = leave_foreign(&s, &config, "Google Chrome", &file);
        let key = key.to_str().unwrap();
        let child = format!(r"{key}\foreign-child");
        let args: Vec<&str> = match extra {
            "child" => vec!["add", &child, "/f"],
            "marker" => vec![
                "add",
                key,
                "/v",
                "vibe-work-removal-lock",
                "/d",
                "foreign",
                "/f",
            ],
            _ => vec!["add", key, "/v", "foreign", "/d", "keep", "/f"],
        };
        assert!(
            std::process::Command::new("reg")
                .args(args)
                .status()
                .unwrap()
                .success()
        );
        let before = registration(key);
        let error = install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap_err();
        assert_eq!(
            error,
            crate::chrome_control::extension_registry::FOREIGN_HOST
        );
        assert_eq!(registration(key), before);
        assert!(config.setup_notice.take().is_none());
    }
}
