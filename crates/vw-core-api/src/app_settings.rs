//! The single writer for the app's settings.json, shared by core features and host switches.
//! Depends on `serde_json` and atomic replacement; preserves unrelated settings and normalizes the scan switch.

use crate::SecretScanSettings;
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

static WRITE: Mutex<()> = Mutex::new(());

/// Merge app settings and atomically save them. The scan switch is saved without fields older versions wrote.
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
    for (key, value) in update {
        if value.is_null() {
            settings.remove(&key);
        } else {
            settings.insert(key, value);
        }
    }
    if let Some(value) = settings.get_mut("secretScan") {
        let scan = saved_scan(value).map_err(std::io::Error::other)?;
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
        .and_then(|settings| saved_scan(settings.get("secretScan")?).ok())
        .unwrap_or_default()
}

/// The saved scan switch. Earlier versions also saved `autoRedact` (automatic redaction, removed): it is dropped and
/// never acts, so such a file keeps its scan setting and other writers can still save. Commands refuse the field.
fn saved_scan(value: &Value) -> serde_json::Result<SecretScanSettings> {
    let mut value = value.clone();
    if let Some(scan) = value.as_object_mut() {
        scan.remove("autoRedact");
    }
    serde_json::from_value(value)
}

/// Missing or older settings keep idle-sleep inhibition on by default.
pub(crate) fn keep_awake_enabled(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|settings| settings.get("keepAwake").and_then(Value::as_bool))
        .unwrap_or(true)
}
