//! Vibe's plugin store stages each file it publishes under `<session folder>/plugins/tmp`, marks it read-only
//! (mode 0o400) and moves it into place; a staging file whose move lost a race to an identical one stays behind, and
//! every session start removes such leftovers once they are a day old. On Windows a read-only file cannot be
//! deleted: that removal failed with "Access is denied" and every new chat failed to start until the file was removed
//! by hand. Before the server starts, the read-only mark is cleared on the staging files left there, so the store's own
//! removal works. Nothing is deleted here and nothing outside that staging folder is touched.
//!
//! Depends on: std only. Called by: `launch` (before the supervisor starts the server).

use std::path::Path;

/// The plugin store's staging folder, relative to a session folder (Vibe 3.1.2 `plugins/_store.py`).
const STAGING: [&str; 2] = ["plugins", "tmp"];

/// Clears the read-only mark of every file directly in `session_dir`'s plugin staging folder (Windows; elsewhere a
/// read-only file in a writable folder can be removed, so nothing is done). A missing folder is fine; a failure is
/// logged and never stops the start.
pub(crate) fn release_leftovers(session_dir: &Path) {
    if cfg!(windows) {
        let staging = STAGING
            .iter()
            .fold(session_dir.to_path_buf(), |p, s| p.join(s));
        let Ok(entries) = std::fs::read_dir(&staging) else {
            return;
        };
        let mut released = 0usize;
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || !meta.permissions().readonly() {
                continue;
            }
            let mut permissions = meta.permissions();
            // On Windows this clears only the read-only attribute.
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            match std::fs::set_permissions(entry.path(), permissions) {
                Ok(()) => released += 1,
                Err(error) => tracing::warn!(%error, "Vibe's plugin staging file stays read-only"),
            }
        }
        if released > 0 {
            tracing::info!(
                released,
                "Vibe's plugin staging: read-only leftovers made removable"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    //! PREDICT (Windows): before the release a leftover staging file keeps its read-only attribute, which makes the
    //! store's removal (Python's `os.unlink`, `DeleteFileW`) fail with "Access is denied"; after it the attribute is
    //! gone. Elsewhere the release changes nothing. (Rust's own `remove_file` ignores the attribute on Windows, so the
    //! attribute itself is what is checked.)
    use super::release_leftovers;

    #[test]
    fn a_read_only_staging_leftover_loses_its_read_only_mark() {
        let session = tempfile::tempdir().unwrap();
        let staging = session.path().join("plugins").join("tmp");
        std::fs::create_dir_all(&staging).unwrap();
        let leftover = staging.join("blob-0123456789abcdef-0011223344556677");
        std::fs::write(&leftover, b"{}").unwrap();
        let mut permissions = std::fs::metadata(&leftover).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&leftover, permissions).unwrap();
        let other = session.path().join("kept.json");
        std::fs::write(&other, b"{}").unwrap();
        let mut permissions = std::fs::metadata(&other).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&other, permissions).unwrap();

        release_leftovers(session.path());

        assert_eq!(
            std::fs::metadata(&leftover)
                .unwrap()
                .permissions()
                .readonly(),
            !cfg!(windows),
            "a Windows leftover loses its read-only mark"
        );
        assert!(
            std::fs::metadata(&other).unwrap().permissions().readonly(),
            "nothing outside the staging folder changes"
        );
        // The temporary folder can be cleaned up afterwards on every platform.
        for path in [&other, &leftover] {
            if let Ok(meta) = std::fs::metadata(path) {
                let mut permissions = meta.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                std::fs::set_permissions(path, permissions).unwrap();
            }
        }
    }

    #[test]
    fn a_missing_staging_folder_is_fine() {
        let session = tempfile::tempdir().unwrap();
        release_leftovers(&session.path().join("absent"));
    }
}
