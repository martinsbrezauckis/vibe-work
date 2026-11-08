//! macOS app-data refusals exercised on Linux with a recording permission boundary and real scratch files.
//! Calls the production setup/probe/install paths; every forbidden attempt returns EPERM before OS I/O.
use super::*;
use std::sync::{Arc, Mutex};

type Calls = Arc<Mutex<Vec<(files::Access, PathBuf)>>>;

fn protected(s: &Scratch) -> (ChromeControlConfig, Calls) {
    let mut config = s.config();
    config.macos = Some(true);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let chrome = s
        .root
        .join("home/Library/Application Support/Google/Chrome");
    let edge = s
        .root
        .join("home/Library/Application Support/Microsoft Edge");
    std::fs::create_dir_all(&chrome).unwrap();
    config.file_access = Some(Arc::new(move |op, path| {
        seen.lock().unwrap().push((op, path.to_owned()));
        if op != files::Access::Traverse
            && [chrome.as_path(), edge.as_path()].iter().any(|root| {
                path.starts_with(root) && !path.starts_with(root.join("NativeMessagingHosts"))
            })
        {
            Err(std::io::Error::from_raw_os_error(1))
        } else {
            Ok(())
        }
    }));
    (config, calls)
}

#[test]
fn macos_host_manifest_uses_the_excluded_directory_without_reading_its_parent() {
    let s = Scratch::new("mac-host-only");
    let (config, calls) = protected(&s);
    let host = targets(&s.root.join("home"), &config)
        .remove(0)
        .hosts
        .unwrap()
        .join(format!("{HOST_NAME}.json"));
    let bytes = host_manifest(&s.state().join("bin").join(helper_name()), EXTENSION_ID);
    registration::ensure_manifest(&config, &host, &bytes, false).unwrap();
    assert_eq!(files::read(&config, &host).unwrap(), Some(bytes.clone()));
    files::remove(&config, &host, &bytes).unwrap();
    assert!(!host.exists());
    assert!(!calls.lock().unwrap().is_empty());
}

#[test]
fn macos_profile_version_and_preferences_probes_make_no_protected_attempt() {
    let s = Scratch::new("mac-no-probes");
    let (config, calls) = protected(&s);
    let data = targets(&s.root.join("home"), &config).remove(0).user_data;
    let probe = super::super::super::probe::Prober::default();
    assert_eq!(
        super::super::super::probe::profiles_for(&config, &data, EXTENSION_ID),
        [] as [PathBuf; 0]
    );
    assert_eq!(
        super::super::super::probe::version_for(&config, &data.join("Default"), EXTENSION_ID),
        None
    );
    assert_eq!(
        probe.probe_for(
            &config,
            vec![data.join("Default")],
            EXTENSION_ID,
            std::time::Instant::now()
        ),
        super::super::super::probe::Probe::Unknown
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [],
        "even a refused attempt raises the macOS alert"
    );
    assert_eq!(probe.started_for_test(), 0);
}

#[test]
fn macos_extension_install_opens_the_store_without_external_registration() {
    let s = Scratch::new("mac-store");
    let (mut config, calls) = protected(&s);
    let opened = Arc::new(Mutex::new(Vec::new()));
    let seen = opened.clone();
    config.launcher = Some(Arc::new(move |url| {
        seen.lock().unwrap().push(url.to_owned());
        Ok(())
    }));
    super::super::super::extension_install::begin(&config, &s.state()).unwrap();
    assert_eq!(*opened.lock().unwrap(), [store_url(EXTENSION_ID)]);
    assert_eq!(*calls.lock().unwrap(), []);
}

#[test]
fn macos_legacy_external_receipt_is_neither_probed_nor_removed() {
    let s = Scratch::new("mac-old-external");
    let (config, calls) = protected(&s);
    std::fs::create_dir_all(s.state()).unwrap();
    super::super::super::extension_install::write_test_registration(&config, &s.state());
    assert_eq!(
        super::super::super::extension_install::status(&config, &s.state()),
        None
    );
    assert!(!super::super::super::extension_install::remove(&config, &s.state()).unwrap());
    assert_eq!(*calls.lock().unwrap(), []);
}

fn bundle(s: &Scratch, browser: &str) {
    use std::os::unix::fs::PermissionsExt;
    let exe = s
        .root
        .join("home/Applications")
        .join(format!("{browser}.app/Contents/MacOS/{browser}"));
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"browser").unwrap();
    std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn macos_chrome_setup_does_not_touch_an_absent_edge() {
    let s = Scratch::new("mac-chrome-only");
    let (config, calls) = protected(&s);
    bundle(&s, "Google Chrome");
    install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
    assert_eq!(
        read_receipt(&config, &s.state()).unwrap().browsers,
        ["Google Chrome"]
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, p)| p.to_string_lossy().contains("Microsoft Edge"))
    );
    assert!(
        !s.root
            .join("home/Library/Application Support/Microsoft Edge")
            .exists()
    );
}

#[cfg(feature = "your-edge")]
#[test]
fn macos_edge_setup_does_not_touch_chrome_even_when_both_are_installed() {
    let s = Scratch::new("mac-edge-only");
    let (config, calls) = protected(&s);
    bundle(&s, "Microsoft Edge");
    bundle(&s, "Google Chrome");
    std::fs::create_dir_all(
        s.root
            .join("home/Library/Application Support/Microsoft Edge"),
    )
    .unwrap();
    install_for(&config, &s.state(), &s.vibe_home(), "Microsoft Edge").unwrap();
    assert_eq!(
        read_receipt(&config, &s.state()).unwrap().browsers,
        ["Microsoft Edge"]
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, p)| p.to_string_lossy().contains("Google/Chrome"))
    );
}

#[test]
fn macos_a_browser_data_folder_does_not_prove_an_installed_browser() {
    let s = Scratch::new("mac-missing-bundle");
    // Presence also checks the machine's own /Applications (vw_platform::browser::macos_executable). On a Mac
    // with Chrome installed the premise "no app bundle" cannot be built, so the refusal is asserted only where it
    // holds (always on Linux, where this fake-macOS test runs in every CI job).
    let probe = s.root.join("no-home");
    if vw_platform::browser::macos_executable(vw_platform::browser::Browser::Chrome, &probe)
        .is_some()
    {
        eprintln!("skipped: this machine has Google Chrome in /Applications");
        return;
    }
    let (config, calls) = protected(&s);
    let error = install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap_err();
    assert_eq!(error, "Google Chrome is not installed");
    assert_eq!(*calls.lock().unwrap(), []);
    assert!(!s.state().exists());
}

#[cfg(feature = "your-edge")]
#[test]
fn macos_a_failed_second_browser_keeps_the_first_registration_and_helper() {
    let s = Scratch::new("mac-rollback-scope");
    let (mut config, _) = protected(&s);
    bundle(&s, "Google Chrome");
    bundle(&s, "Microsoft Edge");
    config.edge_policy = Some(serde_json::json!({"NativeMessagingBlocklist":[HOST_NAME]}));
    install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
    config.edge_policy = None;
    let helper = s.state().join("bin").join(helper_name());
    let host = targets(&s.root.join("home"), &config)
        .remove(0)
        .hosts
        .unwrap()
        .join(format!("{HOST_NAME}.json"));
    let before = [&helper, &host, &s.state().join(RECEIPT)].map(|p| std::fs::read(p).unwrap());
    let underlying = config.file_access.clone().unwrap();
    config.file_access = Some(Arc::new(move |op, path| {
        if path.to_string_lossy().contains("Microsoft Edge") {
            return Err(std::io::Error::from_raw_os_error(1));
        }
        underlying(op, path)
    }));
    assert!(install_for(&config, &s.state(), &s.vibe_home(), "Microsoft Edge").is_err());
    assert_eq!(
        [&helper, &host, &s.state().join(RECEIPT)].map(|p| std::fs::read(p).unwrap()),
        before
    );
}

#[cfg(feature = "your-edge")]
#[test]
fn macos_adding_edge_does_not_repair_or_read_a_broken_chrome_host() {
    let s = Scratch::new("mac-other-broken");
    let (mut config, calls) = protected(&s);
    bundle(&s, "Google Chrome");
    bundle(&s, "Microsoft Edge");
    std::fs::create_dir_all(
        s.root
            .join("home/Library/Application Support/Microsoft Edge"),
    )
    .unwrap();
    config.edge_policy = Some(serde_json::json!({"NativeMessagingBlocklist":[HOST_NAME]}));
    install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
    config.edge_policy = None;
    let underlying = config.file_access.clone().unwrap();
    config.file_access = Some(Arc::new(move |op, path| {
        if path.to_string_lossy().contains("Google/Chrome") {
            return Err(std::io::Error::from_raw_os_error(1));
        }
        underlying(op, path)
    }));
    calls.lock().unwrap().clear();
    register_edge(&config, &s.state()).unwrap();
    assert_eq!(
        read_receipt(&config, &s.state()).unwrap().browsers,
        ["Google Chrome", "Microsoft Edge"]
    );
}

#[path = "macos_status_tests.rs"]
mod status;

#[cfg(feature = "your-edge")]
#[test]
fn macos_remove_skips_an_absent_legacy_edge_even_when_policy_hides_it() {
    let s = Scratch::new("mac-remove-absent");
    let (mut config, _) = protected(&s);
    bundle(&s, "Google Chrome");
    bundle(&s, "Microsoft Edge");
    std::fs::create_dir_all(
        s.root
            .join("home/Library/Application Support/Microsoft Edge"),
    )
    .unwrap();
    install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
    install_for(&config, &s.state(), &s.vibe_home(), "Microsoft Edge").unwrap();
    std::fs::remove_dir_all(s.root.join("home/Applications/Microsoft Edge.app")).unwrap();
    config.edge_policy = Some(serde_json::json!({"NativeMessagingBlocklist":[HOST_NAME]}));
    let underlying = config.file_access.clone().unwrap();
    config.file_access = Some(Arc::new(move |op, path| {
        assert!(
            !path.to_string_lossy().contains("Microsoft Edge"),
            "Remove touched an absent browser: {path:?}"
        );
        underlying(op, path)
    }));
    remove(&config, &s.state()).unwrap();
    assert!(!s.state().exists());
}

#[test]
fn macos_an_unopened_browser_is_asked_to_create_its_own_data_folder() {
    let s = Scratch::new("mac-unopened");
    let (config, calls) = protected(&s);
    bundle(&s, "Google Chrome");
    let data = s
        .root
        .join("home/Library/Application Support/Google/Chrome");
    std::fs::remove_dir(&data).unwrap();
    let error = install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap_err();
    assert!(error.contains("Open Google Chrome once"), "{error}");
    assert!(!data.exists());
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(op, path)| *op == files::Access::CreateDir && path == &data)
    );
}

#[test]
fn macos_native_host_creation_rejects_linked_ancestors_and_host_directories() {
    for hosts_link in [false, true] {
        let s = Scratch::new(if hosts_link {
            "mac-host-link"
        } else {
            "mac-data-link"
        });
        let (config, _) = protected(&s);
        let data = s
            .root
            .join("home/Library/Application Support/Google/Chrome");
        let outside = s.root.join("foreign");
        std::fs::create_dir(&outside).unwrap();
        let link = if hosts_link {
            data.join("NativeMessagingHosts")
        } else {
            std::fs::remove_dir(&data).unwrap();
            data.clone()
        };
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let host = data
            .join("NativeMessagingHosts")
            .join(format!("{HOST_NAME}.json"));
        let bytes = host_manifest(&s.state().join("bin").join(helper_name()), EXTENSION_ID);
        assert!(registration::ensure_manifest(&config, &host, &bytes, false).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    }
}

#[cfg(feature = "your-edge")]
#[test]
fn macos_startup_repair_continues_after_one_browsers_permission_refusal() {
    let s = Scratch::new("mac-startup-repair");
    let (mut config, _) = protected(&s);
    bundle(&s, "Google Chrome");
    bundle(&s, "Microsoft Edge");
    std::fs::create_dir_all(
        s.root
            .join("home/Library/Application Support/Microsoft Edge"),
    )
    .unwrap();
    install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap();
    install_for(&config, &s.state(), &s.vibe_home(), "Microsoft Edge").unwrap();
    let edge = targets(&s.root.join("home"), &config)
        .remove(1)
        .hosts
        .unwrap()
        .join(format!("{HOST_NAME}.json"));
    std::fs::remove_file(&edge).unwrap();
    let underlying = config.file_access.clone().unwrap();
    config.file_access = Some(Arc::new(move |op, path| {
        if path.to_string_lossy().contains("Google/Chrome") {
            return Err(std::io::Error::from_raw_os_error(1));
        }
        underlying(op, path)
    }));
    assert!(repair_registrations(&config, &s.state(), None).is_err());
    assert!(edge.is_file(), "Chrome's refusal must not stop Edge repair");
    assert!(problem::blocked(&config, Some(&s.state()), "Google Chrome"));
    assert!(!problem::blocked(
        &config,
        Some(&s.state()),
        "Microsoft Edge"
    ));
}
