//! Firefox registration on macOS, exercised on Linux through the macOS file flow with real scratch folders.
//! Calls the production install and removal paths; the Firefox bundle is a scratch file in ~/Applications.
//!
//! PREDICT (before the fix): a Mac where Firefox has run still has no `~/Library/Application Support/Mozilla`
//! (Firefox keeps its profiles under `…/Firefox`), so opening the Mozilla folder failed with "No such file or
//! directory" and Connect never registered the helper. After: the missing Mozilla folder is created (owner-only),
//! its `NativeMessagingHosts` folder and manifest follow, and a linked Mozilla folder is still refused.
use super::*;

fn firefox_bundle(s: &Scratch) {
    use std::os::unix::fs::PermissionsExt;
    let exe = s
        .root
        .join("home/Applications/Firefox.app/Contents/MacOS/firefox");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"browser").unwrap();
    std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn mac(s: &Scratch) -> ChromeControlConfig {
    let mut config = s.config();
    config.macos = Some(true);
    std::fs::create_dir_all(
        s.root
            .join("home/Library/Application Support/Firefox/Profiles"),
    )
    .unwrap();
    config
}

#[test]
fn macos_firefox_setup_creates_the_missing_mozilla_folder() {
    use std::os::unix::fs::PermissionsExt;
    let s = Scratch::new("mac-firefox-mozilla");
    let config = mac(&s);
    firefox_bundle(&s);
    let mozilla = s.root.join("home/Library/Application Support/Mozilla");
    assert!(!mozilla.exists());
    install_for(&config, &s.state(), &s.vibe_home(), "Mozilla Firefox").unwrap();
    let mode = std::fs::metadata(&mozilla).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "{mode:o}");
    let manifest = mozilla
        .join("NativeMessagingHosts")
        .join(format!("{HOST_NAME}.json"));
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(
        value["allowed_extensions"],
        serde_json::json!(["browser-control@vibework.app"])
    );
    assert!(registered(&config, Some(&s.state()), "Mozilla Firefox"));
    // An existing Mozilla folder (another add-on's hosts) is reused, never replaced.
    remove_browser(&config, &s.state(), "Mozilla Firefox", true).unwrap();
    std::fs::write(mozilla.join("NativeMessagingHosts/other.json"), b"{}").unwrap();
    install_for(&config, &s.state(), &s.vibe_home(), "Mozilla Firefox").unwrap();
    assert!(mozilla.join("NativeMessagingHosts/other.json").is_file());
}

#[test]
fn macos_firefox_setup_refuses_a_linked_mozilla_folder() {
    let s = Scratch::new("mac-firefox-link");
    let config = mac(&s);
    firefox_bundle(&s);
    let outside = s.root.join("foreign");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(
        &outside,
        s.root.join("home/Library/Application Support/Mozilla"),
    )
    .unwrap();
    assert!(install_for(&config, &s.state(), &s.vibe_home(), "Mozilla Firefox").is_err());
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}
