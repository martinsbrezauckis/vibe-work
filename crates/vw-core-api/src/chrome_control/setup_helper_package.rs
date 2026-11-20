//! The packaged helper: reading it once, checking its release pin, and its native-messaging host manifest.
use super::{
    ABSENT, ChromeControlConfig, HOST_NAME, Path, PathBuf, STORE_LISTING_NAME, helper_name,
    windows_native_path,
};

/// Reads the helper once and checks the release pin before copying or starting it.
pub(in crate::chrome_control) fn packaged_helper(
    config: &ChromeControlConfig,
) -> Result<(PathBuf, Vec<u8>), String> {
    use std::io::Read;
    let (mut opened, path) = open_helper(config)?;
    let mut bytes = Vec::new();
    opened
        .file()
        .read_to_end(&mut bytes)
        .map_err(|_| missing_helper())?;
    check_helper_pin(config, &bytes)?;
    Ok((path, bytes))
}

pub(super) fn missing_helper() -> String {
    vw_vocabulary::texts::CHROME_PACKAGE_MISSING.to_owned()
}

pub(in crate::chrome_control) fn open_helper(
    config: &ChromeControlConfig,
) -> Result<(vw_platform::checked_executable::CheckedExecutable, PathBuf), String> {
    let package = config.package.as_ref().ok_or_else(|| ABSENT.to_owned())?;
    let path = package.join("bin").join(helper_name());
    let opened = vw_platform::checked_executable::CheckedExecutable::open(&path)
        .map_err(|_| missing_helper())?;
    Ok((opened, path))
}

pub(in crate::chrome_control) fn check_helper_pin(
    config: &ChromeControlConfig,
    bytes: &[u8],
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    match config.helper_sha256.as_deref() {
        Some(pin) if format!("{:x}", Sha256::digest(bytes)) == pin => {}
        None if cfg!(debug_assertions) => {}
        _ => {
            return Err(vw_vocabulary::texts::CHROME_PACKAGE_CHANGED.to_owned());
        }
    }
    Ok(())
}

/// The native messaging manifest for the helper at `helper`.
pub(in crate::chrome_control) fn host_manifest(helper: &Path, extension_id: &str) -> Vec<u8> {
    let helper = if cfg!(windows) {
        PathBuf::from(windows_native_path(helper))
    } else {
        helper.to_path_buf()
    };
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": format!("{STORE_LISTING_NAME} extension helper"),
        "path": helper,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")],
    });
    serde_json::to_vec_pretty(&manifest).unwrap_or_default()
}
