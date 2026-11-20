//! Native-messaging host manifests for each browser and where they are written.
use super::{ChromeControlConfig, HOST_NAME, Path, PathBuf, firefox, host_manifest};

// Native messaging expects backslashes even when a configured root uses forward slashes.
pub(super) fn windows_native_path(file: &Path) -> String {
    file.to_string_lossy().replace('/', "\\")
}

pub(super) fn registry_delete(key: &str, file: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    super::super::extension_registry::remove_host(
        key,
        &windows_native_path(file),
        &super::super::extension_registry::Native,
    )
}

pub(super) fn manifest_for(helper: &Path, config: &ChromeControlConfig, browser: &str) -> Vec<u8> {
    if browser == "Mozilla Firefox" {
        firefox::manifest(helper)
    } else {
        host_manifest(helper, &config.extension_id)
    }
}

pub(super) fn manifest_path(state: &Path, browser: &str) -> PathBuf {
    state
        .join(if browser == "Mozilla Firefox" {
            "firefox-host"
        } else {
            "host"
        })
        .join(format!("{HOST_NAME}.json"))
}
