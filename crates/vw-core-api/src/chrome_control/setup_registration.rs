//! Verifies receipted Chrome/Edge host registrations against configured targets. Called by Connect,
//! helper refresh and status; a receipt never authorizes arbitrary paths or another installation's host.
use super::files;
use super::{
    ChromeControlConfig, HOST_NAME, Path, helper_name, home_of, host_manifest, manifest_path,
    read_receipt, targets,
};
use crate::chrome_control::extension_registry::{self, Native};

pub(super) fn repair(
    config: &ChromeControlConfig,
    state: &Path,
    browser: Option<&str>,
) -> Result<(), String> {
    let Some(receipt) = read_receipt(state) else {
        return Ok(());
    };
    let helper = state.join("bin").join(helper_name());
    // A receipt without an installed copy cannot confer registration ownership.
    if !receipt.files.contains(&helper) {
        return Ok(());
    }
    // A linked helper must not become reachable through a freshly repaired registration.
    if !files::plain_file(&helper).map_err(|e| e.to_string())? {
        return Err("Browser helper is missing".into());
    }
    // Chrome can start this copy without the app's launch guard. A receipt alone cannot vouch for its bytes.
    let (_, packaged) = super::packaged_helper(config)?;
    if !files::matches(&helper, &packaged).map_err(|e| e.to_string())? {
        return Err(vw_vocabulary::texts::CHROME_PACKAGE_CHANGED.into());
    }
    let expected = host_manifest(&helper, &config.extension_id);
    let home = home_of(config)?;
    for target in targets(&home, config) {
        if browser.is_some_and(|name| target.name != name)
            || !receipt.browsers.iter().any(|name| name == target.name)
        {
            continue;
        }
        let path = target.hosts.map_or_else(
            || manifest_path(state, target.name),
            |hosts| hosts.join(format!("{HOST_NAME}.json")),
        );
        if !receipt.files.contains(&path) {
            return Err("Browser helper registration is not owned by this setup".into());
        }
        ensure_manifest(&path, &expected, true)?;
        if let Some(root) = target.registry {
            let key = format!(r"{root}\{HOST_NAME}");
            if !receipt.registry_keys.contains(&key) {
                return Err("Browser helper registration is not owned by this setup".into());
            }
            extension_registry::host(&key, &super::windows_native_path(&path), true, &Native)?;
        }
    }
    Ok(())
}

pub(super) fn ensure_manifest(path: &Path, expected: &[u8], owned: bool) -> Result<(), String> {
    let before = files::read(path).map_err(|e| e.to_string())?;
    if let Some(bytes) = &before {
        if !owned {
            return Err("Browser helper is set up by another Vibe Work".into());
        }
        if bytes == expected {
            return Ok(());
        }
        owned_manifest(bytes, expected)?;
    }
    files::write(path, before.as_deref(), expected).map_err(|e| e.to_string())
}

/// The host identity and helper path identify this installation, even when other manifest fields drift.
pub(super) fn owned_manifest(bytes: &[u8], expected: &[u8]) -> Result<(), String> {
    let old: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "Browser helper registration is unrecognized")?;
    let new: serde_json::Value = serde_json::from_slice(expected).map_err(|e| e.to_string())?;
    if old["name"] != new["name"] || old["path"] != new["path"] {
        return Err("Browser helper is set up by another Vibe Work".into());
    }
    Ok(())
}
