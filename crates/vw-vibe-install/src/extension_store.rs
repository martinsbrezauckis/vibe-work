//! The last accepted Chrome extension store state (`chromeExtension.store` of the signed supported-Vibe manifest),
//! kept in `<vibe root>/chrome-extension.json` beside the rollback record (`support-seen.json`), so Your Chrome's
//! setup says "in review" from the start of the next run, before that run's first update check.
//!
//! **Fail open.** The value only replaces the store button with a line, so every doubt reads as absent (the store
//! link: the store page itself says when the item is not available): no file, a file that is not a regular small
//! file, does not parse, or names an unknown state. Only a manifest that passed the signature, schema and rollback
//! checks changes the record ([`crate::update`]); a manifest without the field removes it. A write that fails is
//! logged and the value still applies for this run.
//!
//! Depends on: serde, [`crate::support`]. Called by: [`crate::manager`] (read at start), [`crate::update`] (saved
//! after an accepted manifest).

use std::path::Path;

use crate::support::{ChromeExtension, ExtensionStore};

/// The record's file name under the Vibe root.
pub const EXTENSION_FILE: &str = "chrome-extension.json";
/// Largest record read (a real one is ~20 bytes).
const MAX_BYTES: u64 = 4 * 1024;

/// The recorded state under `root`; `None` for no record or any record that is not a known state.
#[must_use]
pub fn load(root: &Path) -> Option<ExtensionStore> {
    let path = root.join(EXTENSION_FILE);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice::<ChromeExtension>(&bytes)
        .ok()
        .map(|c| c.store)
}

/// Records `store` under `root` (temporary file, then rename); `None` removes the record.
///
/// # Errors
/// The I/O error (the caller logs it; nothing else depends on the record).
pub fn save(root: &Path, store: Option<ExtensionStore>) -> std::io::Result<()> {
    let path = root.join(EXTENSION_FILE);
    let Some(store) = store else {
        return match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    };
    std::fs::create_dir_all(root)?;
    let text = serde_json::to_vec(&ChromeExtension { store }).map_err(std::io::Error::other)?;
    let temp = root.join(format!(
        "{EXTENSION_FILE}.{}.tmp",
        crate::seen::temp_suffix()
    ));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut file| std::io::Write::write_all(&mut file, &text))
        .and_then(|()| std::fs::rename(&temp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}
