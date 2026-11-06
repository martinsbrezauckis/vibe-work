//! Folder reveal admission for the host: current session roots and saved project main folders.
//! Uses platform canonicalization and component containment; neither picked-only folders nor escaped links grant access.

use std::path::{Path, PathBuf};

use crate::{Core, CoreError, ErrorCode};

impl Core {
    /// The existing, local, canonical folder to reveal, inside a current session or project root.
    /// The caller must pass this returned path to the opener, never the original link spelling.
    ///
    /// # Errors
    /// `denied` when no current root contains the resolved folder, or a network/device path is given.
    pub fn reveal_folder_target(&self, path: &Path) -> Result<PathBuf, CoreError> {
        let denied = || {
            CoreError::new(
                ErrorCode::Denied,
                "that folder does not belong to any session or project",
            )
        };
        // Platform validation rejects network/device forms before touching the filesystem and again
        // after resolving links. Compare canonical components, so a sibling sharing a prefix cannot pass.
        let target = vw_platform::reveal_target(path).map_err(|_| denied())?;
        let mut roots = self.shared.names.folders(&self.shared.store);
        roots.extend(self.shared.projects.store.read(|store| {
            store
                .projects()
                .iter()
                .filter_map(|project| project.main_folder.clone())
                .collect::<Vec<_>>()
        }));
        if roots.iter().any(|root| {
            vw_platform::reveal_target(Path::new(root)).is_ok_and(|root| target.starts_with(root))
        }) {
            Ok(target)
        } else {
            Err(denied())
        }
    }
}
