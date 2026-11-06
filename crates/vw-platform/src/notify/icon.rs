//! Persists the toast identity's PNG in app data, independently of installer shortcuts.
//! Called by `DesktopNotifier`; the app supplies the embedded logo and its stable private data path.
use std::io::Read as _;
use std::path::Path;

pub(super) fn save(path: &Path, png: &[u8]) -> Result<(), super::NotifyError> {
    let parent = path
        .parent()
        .ok_or_else(|| super::NotifyError("icon path has no parent".into()))?;
    crate::ensure_private_dir(parent).map_err(|e| super::NotifyError(e.to_string()))?;
    let existing = std::fs::File::open(path).and_then(|file| {
        let mut bytes = Vec::new();
        file.take(png.len() as u64 + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    });
    if existing.ok().as_deref() == Some(png) {
        return Ok(());
    }
    crate::replace_private(path, png).map_err(|e| super::NotifyError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_logo_is_repaired_after_upgrade_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app/notification-icon.png");
        let logo = b"\x89PNG\r\n\x1a\nembedded-logo";
        save(&path, logo).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), logo);
        save(&path, logo).unwrap();
        std::fs::write(&path, b"old-logo").unwrap();
        save(&path, logo).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), logo);
    }
}
