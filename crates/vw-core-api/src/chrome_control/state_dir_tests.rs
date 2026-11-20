//! Where the helper keeps its state (pairing, pins, `state.json`, `safari-enabled.json`, `broker.sock`).
//!
//! PREDICT (red before the change): a development build on macOS that carries the Safari parts used the installed,
//! signed app's folder (`~/Library/Application Support/VibeWork/ChromeControl`), so a debug Safari run rewrote the
//! signed app's helper state (its pins: the signed helper then exited with an owner identity mismatch). There was no
//! `dev_build` switch and no way to name another folder, so this file does not compile before the change.

use std::path::{Path, PathBuf};

use super::setup::ChromeControlConfig;
use super::{macos_state_dir, state_dir};

/// The bytes of the signed release folder, as the helper's own default (`broker/src/main.rs`) spells it.
#[cfg(target_os = "macos")]
const RELEASE_MACOS: &str = "/Users/me/Library/Application Support/VibeWork/ChromeControl";

#[cfg(target_os = "macos")]
#[test]
fn the_signed_release_folder_on_macos_is_unchanged_byte_for_byte() {
    let release = macos_state_dir(Path::new("/Users/me"), false);
    assert_eq!(
        release.as_os_str().as_encoded_bytes(),
        RELEASE_MACOS.as_bytes()
    );
}

#[test]
fn a_debug_safari_build_on_macos_has_its_own_folder_beside_the_release_one() {
    let dev = macos_state_dir(Path::new("/Users/me"), true);
    assert_eq!(
        dev,
        PathBuf::from("/Users/me/Library/Application Support/VibeWork/ChromeControl-dev")
    );
    assert_ne!(dev, macos_state_dir(Path::new("/Users/me"), false));
    // Neither folder holds the other: removing or resetting one never touches the other's files.
    assert!(!dev.starts_with(macos_state_dir(Path::new("/Users/me"), false)));
}

#[test]
fn a_scratch_home_keeps_its_disposable_folder_in_every_build() {
    let config = |dev_build| ChromeControlConfig {
        package: Some(PathBuf::from("/pkg/chrome-control")),
        user_home: Some(PathBuf::from("/scratch/home")),
        dev_build,
        ..ChromeControlConfig::default()
    };
    let expected = Some(PathBuf::from("/scratch/home/VibeWork/ChromeControl"));
    assert_eq!(state_dir(&config(false)), expected);
    assert_eq!(state_dir(&config(true)), expected);
}

#[test]
fn the_release_config_never_asks_for_the_development_folder() {
    assert!(
        !ChromeControlConfig::default().dev_build,
        "only the app's debug build sets dev_build"
    );
}

// Chrome's native host (started by Chrome, no arguments) always uses the default folder, so off macOS, and on macOS
// without the Safari parts, a development build keeps the default folder: its Chrome would otherwise never connect.
#[cfg(not(target_os = "macos"))]
#[test]
fn off_macos_a_development_build_keeps_the_default_folder() {
    let config = |dev_build| ChromeControlConfig {
        package: Some(PathBuf::from("/pkg/chrome-control")),
        dev_build,
        ..ChromeControlConfig::default()
    };
    assert_eq!(state_dir(&config(true)), state_dir(&config(false)));
}
