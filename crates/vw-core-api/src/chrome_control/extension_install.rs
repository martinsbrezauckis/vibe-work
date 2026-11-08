//! Requests Chrome's own extension consent on Windows; macOS and Linux use the Store.
//! Setup owns only registrations it creates, never Chrome profile preferences or enterprise policies.
use std::path::{Path, PathBuf};

use super::extension_registry;
use super::setup::{self, ChromeControlConfig};

const UPDATE_URL: &str = "https://clients2.google.com/service/update2/crx";
const RECEIPT: &str = "extension-install.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct Registration {
    file: Option<PathBuf>,
    key: Option<String>,
    /// macOS does not watch the external directory: these processes must finish before the next scan.
    chrome_pids: Vec<String>,
}

fn bytes() -> Vec<u8> {
    serde_json::to_vec_pretty(&serde_json::json!({"external_update_url":UPDATE_URL}))
        .unwrap_or_default()
}

fn read(state: &Path) -> Option<Registration> {
    serde_json::from_slice(&super::probe::read_regular(
        &state.join(RECEIPT),
        16 * 1024,
    )?)
    .ok()
}

fn manifest_path(config: &ChromeControlConfig) -> Result<PathBuf, String> {
    Ok(setup::home_of(config)?
        .join("Library/Application Support/Google/Chrome/External Extensions")
        .join(format!("{}.json", config.extension_id)))
}

fn registry_key(config: &ChromeControlConfig) -> Result<String, String> {
    let root = config
        .windows_registry_root
        .strip_suffix("NativeMessagingHosts")
        .filter(|root| root.starts_with("HKCU\\"))
        .ok_or("Chrome registry root unavailable")?;
    Ok(format!("{root}Extensions\\{}", config.extension_id))
}

fn permitted(config: &ChromeControlConfig, registration: &Registration) -> Result<(), String> {
    // Compare the serialized spelling too: Path equality normalizes redundant separators and `.`.
    if let Some(file) = &registration.file
        && file.as_os_str() != manifest_path(config)?.as_os_str()
    {
        return Err("Chrome registration receipt does not match this app".into());
    }
    if let Some(key) = &registration.key
        && *key != registry_key(config)?
    {
        return Err("Chrome registration receipt does not match this app".into());
    }
    Ok(())
}

fn chrome_pids(config: &ChromeControlConfig) -> Option<Vec<String>> {
    if !config.is_macos() {
        return Some(Vec::new());
    }
    super::browser::snapshot(config, vw_platform::browser::Browser::Chrome)?
        .process_ids
        .map(|ids| ids.into_iter().map(|id| id.to_string()).collect())
}

pub(super) fn status(config: &ChromeControlConfig, state: &Path) -> Option<String> {
    if config.is_macos() {
        return None;
    }
    let registration = read(state)?;
    let live = chrome_pids(config);
    Some(
        if !registration.chrome_pids.is_empty()
            && live.as_ref().is_none_or(|live| {
                registration
                    .chrome_pids
                    .iter()
                    .any(|pid| live.contains(pid))
            })
        {
            "restartChrome"
        } else {
            "external"
        }
        .to_owned(),
    )
}

/// Run setup's OS/file work away from async request workers, after the first native observation.
pub(super) async fn begin_async(config: &ChromeControlConfig, state: &Path) -> Result<(), String> {
    super::browser::refresh(config).await;
    let config = config.clone();
    let state = state.to_owned();
    tokio::task::spawn_blocking(move || begin(&config, &state))
        .await
        .map_err(|_| "The browser setup task ended".to_owned())?
}

/// Use a native consent prompt where supported. An existing registration uses the Store for recovery,
/// including after a person removed an externally installed extension in Chrome (Chrome remembers that).
pub(super) fn begin(config: &ChromeControlConfig, state: &Path) -> Result<(), String> {
    if config.is_macos() || !cfg!(windows) || read(state).is_some() {
        return setup::open_store(config);
    }
    // Isolated configurations derive the sibling within their disposable HKCU root.
    let key = registry_key(config)?;
    if !extension_registry::create(&key, &extension_registry::Native)? {
        return setup::open_store(config);
    }
    let registration = Registration {
        file: None,
        key: Some(key),
        chrome_pids: chrome_pids(config).unwrap_or_default(),
    };
    let saved =
        serde_json::to_vec(&registration).map_err(|_| "Chrome registration failed".to_owned())?;
    if vw_platform::write_private_new(&state.join(RECEIPT), &saved).is_err() {
        undo(config, &registration)?;
        return Err("Chrome registration failed".to_owned());
    }
    prompt(config, super::browser::running(config))
}

fn prompt(config: &ChromeControlConfig, running: bool) -> Result<(), String> {
    if running {
        // Chrome watches the registration and presents its own enable prompt.
        Ok(())
    } else {
        super::browser::open(config, "")
    }
}

fn undo(config: &ChromeControlConfig, registration: &Registration) -> Result<bool, String> {
    let mut removed = false;
    if let Some(file) = &registration.file {
        setup::files::access(
            config,
            setup::files::Access::ReadDir,
            file.parent().ok_or("Chrome folder unavailable")?,
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(file) = &registration.file
        && super::probe::read_regular(file, 1024).is_some_and(|current| current == bytes())
    {
        std::fs::remove_file(file).map_err(|_| "Chrome registration removal failed".to_owned())?;
        removed = true;
    }
    Ok(
        extension_registry::undo(registration.key.as_deref(), &extension_registry::Native)?
            || removed,
    )
}

/// Undoes this flow's registration. `Ok(true)`: one was undone, so Chrome removes the extension it installed from
/// it by itself.
pub(super) fn remove(config: &ChromeControlConfig, state: &Path) -> Result<bool, String> {
    // Old external registrations are inside protected browser data. The person removes the extension in Chrome.
    if config.is_macos() {
        return Ok(false);
    }
    if let Some(registration) = read(state) {
        permitted(config, &registration)?;
        return undo(config, &registration);
    }
    Ok(false)
}

/// Tests: this app's own file registration and its receipt, as Connect leaves them for an external install.
#[cfg(test)]
pub(super) fn write_test_registration(config: &ChromeControlConfig, state: &Path) {
    let file = manifest_path(config).unwrap();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, bytes()).unwrap();
    let registration = Registration {
        file: Some(file),
        key: None,
        chrome_pids: vec![],
    };
    std::fs::write(
        state.join(RECEIPT),
        serde_json::to_vec(&registration).unwrap(),
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn tampered_receipt_cannot_remove_an_unrelated_empty_hkcu_key() {
        let root = std::env::temp_dir().join(format!("receipt-key-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let key = format!(r"Software\VibeWorkTests\receipt-{}", std::process::id());
        let (handle, _) = winreg::HKCU.create_subkey(&key).unwrap();
        drop(handle);
        let registration = Registration {
            file: None,
            key: Some(format!(r"HKCU\{key}")),
            chrome_pids: vec![],
        };
        std::fs::write(
            root.join(RECEIPT),
            serde_json::to_vec(&registration).unwrap(),
        )
        .unwrap();
        assert!(remove(&ChromeControlConfig::default(), &root).is_err());
        assert!(winreg::HKCU.open_subkey(&key).is_ok());
        winreg::HKCU.delete_subkey(&key).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn removal_does_not_follow_a_manifest_symlink() {
        let root =
            std::env::temp_dir().join(format!("external-manifest-link-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let config = ChromeControlConfig {
            user_home: Some(root.clone()),
            ..Default::default()
        };
        let target = root.join("unrelated.json");
        std::fs::write(&target, bytes()).unwrap();
        let file = manifest_path(&config).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        let registration = Registration {
            file: Some(file.clone()),
            key: None,
            chrome_pids: vec![],
        };
        std::fs::write(
            root.join(RECEIPT),
            serde_json::to_vec(&registration).unwrap(),
        )
        .unwrap();
        assert!(!remove(&config, &root).unwrap());
        assert_eq!(std::fs::read(target).unwrap(), bytes());
        assert!(file.is_symlink());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn receipt_destinations_must_match_configuration_exactly() {
        let config = ChromeControlConfig {
            user_home: Some(std::env::temp_dir()),
            windows_registry_root: r"HKCU\Software\VibeWorkTests\receipt\NativeMessagingHosts"
                .into(),
            ..Default::default()
        };
        let mut registration = Registration {
            file: Some(manifest_path(&config).unwrap()),
            key: Some(registry_key(&config).unwrap()),
            chrome_pids: vec![],
        };
        assert!(permitted(&config, &registration).is_ok());
        registration.key = Some(r"HKCU\Software\VibeWorkTests\unrelated".into());
        assert!(permitted(&config, &registration).is_err());
        registration.key = Some(registry_key(&config).unwrap().to_lowercase());
        assert!(permitted(&config, &registration).is_err());
        registration.key = None;
        registration.file = Some(
            config
                .user_home
                .as_ref()
                .unwrap()
                .join("./Library/Application Support/Google/Chrome/External Extensions")
                .join(format!("{}.json", config.extension_id)),
        );
        assert!(permitted(&config, &registration).is_err());
    }

    #[test]
    fn oversized_receipts_are_not_read() {
        let root =
            std::env::temp_dir().join(format!("external-receipt-large-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut data = br#"{"file":null,"key":null,"chrome_pids":[]}"#.to_vec();
        data.resize(16 * 1024 + 1, b' ');
        std::fs::write(root.join(RECEIPT), data).unwrap();
        assert!(read(&root).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removal_refuses_a_receipt_naming_an_unrelated_file() {
        let root = std::env::temp_dir().join(format!("external-receipt-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("unrelated.json");
        std::fs::write(&target, bytes()).unwrap();
        let config = ChromeControlConfig {
            user_home: Some(root.clone()),
            ..Default::default()
        };
        let registration = Registration {
            file: Some(target.clone()),
            key: None,
            chrome_pids: vec![],
        };
        std::fs::write(
            root.join(RECEIPT),
            serde_json::to_vec(&registration).unwrap(),
        )
        .unwrap();
        let _ = remove(&config, &root);
        assert!(
            target.exists(),
            "an editable receipt cannot authorize another destination"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn receipt_symlinks_are_not_read() {
        let root =
            std::env::temp_dir().join(format!("external-receipt-link-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("elsewhere.json");
        std::fs::write(&target, br#"{"file":null,"key":null,"chrome_pids":[]}"#).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, root.join(RECEIPT)).unwrap();
        #[cfg(windows)]
        if !crate::read_exclusions::tests::test_link_created(
            std::os::windows::fs::symlink_file(&target, root.join(RECEIPT)),
            "chrome_control::extension_install::tests::receipt_symlinks_are_not_read",
        ) {
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        assert!(read(&root).is_none(), "never follow the receipt symlink");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_consent_starts_chrome_only_when_needed_without_an_internal_url() {
        use std::sync::{Arc, Mutex};
        for running in [false, true] {
            let opened = Arc::new(Mutex::new(Vec::new()));
            let seen = opened.clone();
            let config = ChromeControlConfig {
                launcher: Some(Arc::new(move |url| {
                    seen.lock().unwrap().push(url.to_owned());
                    Ok(())
                })),
                ..ChromeControlConfig::default()
            };
            prompt(&config, running).unwrap();
            let expected: Vec<&str> = if running { vec![] } else { vec![""] };
            assert_eq!(*opened.lock().unwrap(), expected, "running={running}");
        }
    }

    #[test]
    fn changed_legacy_registration_files_survive_removal() {
        let root = std::env::temp_dir().join(format!("chrome-external-{}", std::process::id()));
        let config = &ChromeControlConfig::default();
        let path = root.join("external.json");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&path, bytes()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes());
        let registration = Registration {
            file: Some(path.clone()),
            key: None,
            chrome_pids: vec![],
        };
        std::fs::write(&path, b"another installer").unwrap();
        undo(config, &registration).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"another installer");
        std::fs::write(&path, bytes()).unwrap();
        undo(config, &registration).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
