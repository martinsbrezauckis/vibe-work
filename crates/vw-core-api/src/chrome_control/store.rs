//! The Browser setting stores availability only. Legacy permissions files are never read.
//! Called by `ChromeControl`; action authority always comes from the current chat mode.
use std::path::Path;
pub(super) fn enabled(dir: &Path) -> bool {
    match std::fs::read(dir.join("enabled.json")) {
        Ok(bytes) => serde_json::from_slice::<bool>(&bytes).unwrap_or(false),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}
#[cfg(test)]
pub(super) fn save_enabled(dir: &Path, enabled: bool) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|_| "Could not save the setting. Nothing changed; try again.")?;
    std::fs::write(
        dir.join("enabled.json"),
        if enabled { "true" } else { "false" },
    )
    .map_err(|_| "Could not save the setting. Nothing changed; try again.".to_owned())
}

/// A profile was connected with this setup at least once. Missing or corrupt = no.
pub(super) fn paired_once(dir: &Path) -> bool {
    std::fs::read(dir.join("paired.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<bool>(&bytes).ok())
        .unwrap_or(false)
}
pub(super) fn save_paired_once(dir: &Path, paired: bool) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("paired.json"),
        if paired { "true" } else { "false" },
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leftover_permissions_never_change_availability_or_authority() {
        let dir = std::env::temp_dir().join(format!("chrome-setting-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for value in ["broken", "{\"level\":\"act\"}", "{\"level\":\"browse\"}"] {
            std::fs::write(dir.join("permissions.json"), value).unwrap();
            assert!(enabled(&dir));
            assert_eq!(
                vw_ask::chrome::Tier::from_mode("ask"),
                Some(vw_ask::chrome::Tier::Ask)
            );
        }
        save_enabled(&dir, false).unwrap();
        assert!(!enabled(&dir));
        std::fs::write(dir.join("enabled.json"), "broken").unwrap();
        assert!(!enabled(&dir));
        std::fs::write(dir.join("local-network.json"), "false").unwrap();
        assert!(
            !enabled(&dir),
            "a retired network setting does not alter availability"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// The first per-browser write freezes both legacy choices together, so later changes cannot recouple them.
#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Switches {
    pub chrome: bool,
    pub edge: bool,
}
impl Switches {
    pub(super) fn load(dir: Option<&Path>) -> Self {
        let Some(dir) = dir else {
            return Self {
                chrome: true,
                edge: true,
            };
        };
        match std::fs::read(dir.join("browser-enabled.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let legacy = enabled(dir);
                Self {
                    chrome: legacy,
                    edge: legacy,
                }
            }
            Err(_) => Self::default(),
        }
    }
    pub(super) fn save(self, dir: &Path) -> Result<(), String> {
        let failed = || "Could not save the setting. Nothing changed; try again.".to_owned();
        std::fs::create_dir_all(dir).map_err(|_| failed())?;
        let bytes = serde_json::to_vec(&self).map_err(|_| failed())?;
        vw_platform::replace_private(&dir.join("browser-enabled.json"), &bytes)
            .map_err(|_| failed())
    }
}
