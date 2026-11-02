//! The Browser setting stores availability only. Legacy permissions files are never read.
//! Called by `ChromeControl`; action authority always comes from the current chat mode.
use std::path::Path;
pub(super) fn enabled(dir: &Path) -> bool {
    match std::fs::read(dir.join("enabled.json")) {
        Ok(bytes) => serde_json::from_slice::<bool>(&bytes).unwrap_or(false),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}
pub(super) fn save_enabled(dir: &Path, enabled: bool) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|_| "Could not save the setting. Nothing changed; try again.")?;
    std::fs::write(
        dir.join("enabled.json"),
        if enabled { "true" } else { "false" },
    )
    .map_err(|_| "Could not save the setting. Nothing changed; try again.".to_owned())
}

/// Settings > Developer > Local network pages for the agent (Chrome Control only). Missing or corrupt = off.
pub(super) fn local_network(dir: &Path) -> bool {
    std::fs::read(dir.join("local-network.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<bool>(&bytes).ok())
        .unwrap_or(false)
}
pub(super) fn save_local_network(dir: &Path, on: bool) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|_| "Could not save the setting. Nothing changed; try again.")?;
    std::fs::write(
        dir.join("local-network.json"),
        if on { "true" } else { "false" },
    )
    .map_err(|_| "Could not save the setting. Nothing changed; try again.".to_owned())
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
        assert!(!local_network(&dir), "absent: off");
        std::fs::write(dir.join("local-network.json"), "yes").unwrap();
        assert!(!local_network(&dir), "corrupt: off");
        save_local_network(&dir, true).unwrap();
        assert!(local_network(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
