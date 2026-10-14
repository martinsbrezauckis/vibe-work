//! The app's one vault selection record and discovery across its private vault and two shared layouts.
//!
//! Depends on `layout`, `repo` and `private_location`; called by `Vault`. Startup relocates the older
//! private profile with verification before rewriting its choice; shared choices remain binding.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::VaultError;
use crate::layout::{Detected, LocalVault, detect};
use crate::location::VaultLocation;
use crate::private_location::{Migration, check_path, private_vault_dir};
use crate::repo::{create_private_dir_all, write_synced};
use crate::vault::VaultKind;

fn private_vault_name() -> &'static str {
    vw_vocabulary::label("vaultKinds", "vibeWork")
}

/// The separate private profile, older location and install's selection record.
pub(crate) struct ManagedVault {
    pub(crate) private_dir: PathBuf,
    pub(crate) legacy_dir: PathBuf,
    selection_path: PathBuf,
}

/// A local vault the app can offer to the user.
pub(crate) struct AvailableVault {
    pub(crate) kind: VaultKind,
    pub(crate) path: PathBuf,
    pub(crate) vault: LocalVault,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SelectionRecord {
    kind: VaultKind,
    path: PathBuf,
}

impl ManagedVault {
    pub(crate) fn new(data_dir: PathBuf) -> Result<Self, VaultError> {
        let private_dir = private_vault_dir(&data_dir)?;
        let legacy_dir = data_dir.join("vault");
        let mut selection_path = data_dir;
        selection_path.push("vault-selection.json");
        Ok(Self {
            private_dir,
            legacy_dir,
            selection_path,
        })
    }

    pub(crate) fn validate_paths(&self) -> Result<(), VaultError> {
        let data_dir = self.selection_path.parent().ok_or_else(|| {
            VaultError::Corrupt(format!("{} has no app data folder", private_vault_name()))
        })?;
        for path in [
            data_dir,
            &self.private_dir,
            &self.legacy_dir,
            &self.selection_path,
        ] {
            check_path(path)?;
        }
        Ok(())
    }

    pub(crate) fn prepare(&self, shared: &VaultLocation) -> Result<(), VaultError> {
        let mut migration = Migration::begin(&self.legacy_dir, &self.private_dir)?;
        // Read and validate the choice before moving anything. A malformed record is never replaced.
        let record = self.read_record(shared)?;
        if self
            .legacy_dir
            .try_exists()
            .map_err(|e| VaultError::io("find the legacy vault", &self.legacy_dir, e))?
            && !matches!(
                detect(&VaultLocation::explicit(&self.legacy_dir))?,
                Detected::One(LocalVault::Shellx { .. })
            )
        {
            // An unrelated partial private folder cannot block an already chosen shared vault.
            if record
                .as_ref()
                .is_some_and(|r| r.kind != VaultKind::VibeWork)
            {
                return Ok(());
            }
            return Err(VaultError::Corrupt(format!(
                "old {} is incomplete; it was kept",
                private_vault_name()
            )));
        }
        migration.relocate()?;
        let publish = (|| {
            if record
                .as_ref()
                .is_some_and(|r| r.kind == VaultKind::VibeWork && r.path == self.legacy_dir)
            {
                if !migration.relocated() {
                    return Err(VaultError::SelectedVaultMissing);
                }
                self.relocate_record()
            } else if record.is_none()
                && self
                    .available(shared)?
                    .iter()
                    .any(|entry| entry.kind == VaultKind::VibeWork)
            {
                self.save(shared, VaultKind::VibeWork)
            } else {
                Ok(())
            }
        })();
        if let Err(error) = publish {
            migration.rollback()?;
            return Err(error);
        }
        migration.finish()
    }

    fn relocate_record(&self) -> Result<(), VaultError> {
        let body = serde_json::to_vec_pretty(&SelectionRecord {
            kind: VaultKind::VibeWork,
            path: self.private_dir.clone(),
        })
        .map_err(|e| VaultError::Corrupt(e.to_string()))?;
        let tmp = self.selection_path.with_file_name(format!(
            ".vault-relocation-{}.tmp",
            hex::encode(vault_core::random_bytes::<8>())
        ));
        write_synced(&tmp, &body)?;
        let result = std::fs::rename(&tmp, &self.selection_path).map_err(|e| {
            VaultError::io("update relocated vault selection", &self.selection_path, e)
        });
        let _ = std::fs::remove_file(&tmp);
        result
    }

    pub(crate) fn path_for(&self, shared: &VaultLocation, kind: VaultKind) -> PathBuf {
        match kind {
            VaultKind::VibeWork => self.private_dir.clone(),
            VaultKind::Shellx | VaultKind::Desktop => shared.profile_dir.clone(),
        }
    }

    pub(crate) fn selected(&self, shared: &VaultLocation) -> Result<Option<VaultKind>, VaultError> {
        Ok(self.read_record(shared)?.map(|record| record.kind))
    }

    fn read_record(&self, shared: &VaultLocation) -> Result<Option<SelectionRecord>, VaultError> {
        let raw = match std::fs::read(&self.selection_path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(VaultError::io(
                    "read vault selection",
                    &self.selection_path,
                    error,
                ));
            }
        };
        let record: SelectionRecord = serde_json::from_slice(&raw)
            .map_err(|_| VaultError::Corrupt("vault selection is invalid".to_owned()))?;
        if record.path != self.path_for(shared, record.kind)
            && !(record.kind == VaultKind::VibeWork && record.path == self.legacy_dir)
        {
            return Err(VaultError::Corrupt(
                "vault selection location changed".to_owned(),
            ));
        }
        Ok(Some(record))
    }

    pub(crate) fn save(&self, shared: &VaultLocation, kind: VaultKind) -> Result<(), VaultError> {
        let path = self.path_for(shared, kind);
        let body = serde_json::to_vec_pretty(&SelectionRecord { kind, path })
            .map_err(|error| VaultError::Corrupt(error.to_string()))?;
        let dir = self.selection_path.parent().ok_or_else(|| {
            VaultError::Corrupt("vault selection has no parent folder".to_owned())
        })?;
        create_private_dir_all(dir)?;
        let tmp = dir.join(format!(
            ".vault-selection-{}.tmp",
            hex::encode(vault_core::random_bytes::<8>())
        ));
        write_synced(&tmp, &body)?;
        // Create once: two app processes choosing different vaults cannot replace each other's
        // record. A concurrent identical choice is harmless; a different choice is refused.
        let publish = std::fs::hard_link(&tmp, &self.selection_path);
        let _ = std::fs::remove_file(&tmp);
        match publish {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if self.selected(shared)? == Some(kind) {
                    Ok(())
                } else {
                    Err(VaultError::SelectionMismatch)
                }
            }
            Err(error) => Err(VaultError::io(
                "save vault selection",
                &self.selection_path,
                error,
            )),
        }
    }

    pub(crate) fn available(
        &self,
        shared: &VaultLocation,
    ) -> Result<Vec<AvailableVault>, VaultError> {
        let mut found = Vec::new();
        let private = VaultLocation::explicit(&self.private_dir);
        let private_detection = detect(&private);
        let selected_shared = matches!(
            self.selected(shared)?,
            Some(VaultKind::Shellx | VaultKind::Desktop)
        );
        // An already selected shared vault must keep working when an unrelated private folder is
        // damaged. Before selection, damaged private data blocks creation over that folder.
        if !selected_shared
            || matches!(
                &private_detection,
                Ok(Detected::One(LocalVault::Shellx { .. }))
            )
        {
            match private_detection? {
                Detected::One(vault @ LocalVault::Shellx { .. }) => found.push(AvailableVault {
                    kind: VaultKind::VibeWork,
                    path: self.private_dir.clone(),
                    vault,
                }),
                Detected::None => {
                    // A prior creation may have written encrypted repository data before its profile.
                    // Treat that as damaged, never initialize another keyfile over the same folder.
                    match std::fs::read_dir(&self.private_dir) {
                        Ok(mut entries) => {
                            if entries.next().is_some() {
                                return Err(VaultError::Corrupt(format!(
                                    "{} folder has data but no profile",
                                    private_vault_name()
                                )));
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(VaultError::io(
                                "inspect private vault folder",
                                &self.private_dir,
                                error,
                            ));
                        }
                    }
                }
                _ => {
                    return Err(VaultError::Corrupt(format!(
                        "{} folder has an unsupported layout",
                        private_vault_name()
                    )));
                }
            }
        }
        // The app-owned vault is independent of the shared profile. A damaged or inaccessible
        // shared profile is not a reason to prevent creation or unlock of the private one.
        match detect(shared).unwrap_or(Detected::None) {
            Detected::One(vault @ LocalVault::Shellx { .. }) => found.push(AvailableVault {
                kind: VaultKind::Shellx,
                path: shared.profile_dir.clone(),
                vault,
            }),
            Detected::One(vault @ LocalVault::Desktop { .. }) => found.push(AvailableVault {
                kind: VaultKind::Desktop,
                path: shared.profile_dir.clone(),
                vault,
            }),
            Detected::Two { shellx, desktop } => {
                found.push(AvailableVault {
                    kind: VaultKind::Shellx,
                    path: shared.profile_dir.clone(),
                    vault: shellx,
                });
                found.push(AvailableVault {
                    kind: VaultKind::Desktop,
                    path: shared.profile_dir.clone(),
                    vault: desktop,
                });
            }
            Detected::None
            | Detected::LegacyOnly
            | Detected::SetupUnfinished
            | Detected::Remote => {}
        }
        Ok(found)
    }

    pub(crate) fn has_record(&self) -> bool {
        Path::new(&self.selection_path).is_file()
    }
}
