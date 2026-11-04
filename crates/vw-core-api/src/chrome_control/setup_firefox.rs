//! Retained legacy manifest/removal compatibility. The owned Firefox window creates no native registration.
//! Firefox's `allowed_extensions` identity is independent of Chromium origins; no profile preference is changed.
use super::{FIREFOX_EXTENSION_ID, HOST_NAME, Path};

pub(super) fn manifest(helper: &Path) -> Vec<u8> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "name": HOST_NAME, "description": "Vibe Work extension helper", "path": helper,
        "type": "stdio", "allowed_extensions": [FIREFOX_EXTENSION_ID],
    }))
    .unwrap_or_default()
}

// A foreign native registration is never replaced or added to our removal receipt.
pub(super) fn require_unregistered(target: &super::places::Target<'_>) -> Result<(), String> {
    if let Some(hosts) = &target.hosts {
        if hosts
            .join(format!("{HOST_NAME}.json"))
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Err("Firefox is already set up by another Vibe Work".into());
        }
    } else if let Some(root) = target.registry {
        let result = std::process::Command::new("reg")
            .args(["query", &format!(r"{root}\{HOST_NAME}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| e.to_string())?;
        if result.success() {
            return Err("Firefox is already set up by another Vibe Work".into());
        }
        if result.code() != Some(1) {
            return Err("Firefox registration could not be checked".into());
        }
    }
    Ok(())
}
