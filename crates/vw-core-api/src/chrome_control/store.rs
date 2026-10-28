//! The saved Chrome Control level: `permissions.json` in the state folder, atomically written with
//! `replace_private` (user-only). A record is untrusted state: the parser checks its shape and version only.
//! Missing, unreadable or older records read as Browse. `restore` requires native person-only consent before a
//! parsed level above Browse becomes effective, even when the JSON has exactly the shape this version writes.
//! Depends on: vw-platform (private files), `vw_ask::chrome`. Called by: `chrome_control`.
use std::path::Path;

use vw_ask::ChromeSettings;
use vw_ask::chrome::Level;

const FILE: &str = "permissions.json";
const VERSION: u32 = 2;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    level: Level,
}

/// The untrusted saved level, or the default when its shape/version cannot be read.
pub(super) fn load(dir: &Path) -> ChromeSettings {
    let path = dir.join(FILE);
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Saved>(&bytes) {
            Ok(saved) if saved.version == VERSION => ChromeSettings { level: saved.level },
            _ => {
                tracing::warn!("Chrome Control's saved level could not be read; using the default");
                ChromeSettings::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ChromeSettings::default(),
        Err(e) => {
            tracing::warn!(%e, "Chrome Control's saved level could not be read; using the default");
            ChromeSettings::default()
        }
    }
}

/// Writes the level atomically.
pub(super) fn save(dir: &Path, settings: ChromeSettings) -> Result<(), String> {
    vw_platform::ensure_private_dir(dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&Saved {
        version: VERSION,
        level: settings.level,
    })
    .map_err(|e| e.to_string())?;
    vw_platform::replace_private(&dir.join(FILE), &bytes).map_err(|e| e.to_string())
}
