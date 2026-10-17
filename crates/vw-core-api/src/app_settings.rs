//! The single writer for the app's settings.json, shared by core features and host switches.
//! Depends on `serde_json` and atomic replacement; preserves unrelated settings and normalizes scan switches.

use crate::SecretScanSettings;
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

static WRITE: Mutex<()> = Mutex::new(());

/// Merge app settings and atomically save them. Disabling scanning always saves automatic redaction off.
///
/// # Errors
/// Settings cannot be read, parsed, or written. No Vibe configuration is touched.
pub fn write_app_settings(path: &Path, update: Map<String, Value>) -> std::io::Result<()> {
    let _write = WRITE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut settings = match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice::<Map<String, Value>>(&bytes).map_err(std::io::Error::other)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(error) => return Err(error),
    };
    let was_enabled = settings
        .get("secretScan")
        .and_then(|scan| scan.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for (key, value) in update {
        if value.is_null() {
            settings.remove(&key);
        } else {
            settings.insert(key, value);
        }
    }
    if let Some(value) = settings.get_mut("secretScan") {
        let mut scan: SecretScanSettings =
            serde_json::from_value(value.clone()).map_err(std::io::Error::other)?;
        scan.auto_redact &= scan.enabled && was_enabled;
        *value = serde_json::to_value(scan).map_err(std::io::Error::other)?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(&settings).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(temp, path)
}

pub(crate) fn scan_settings(path: &Path) -> SecretScanSettings {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|settings| serde_json::from_value(settings.get("secretScan")?.clone()).ok())
        .unwrap_or_default()
}
