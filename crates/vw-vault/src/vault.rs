//! `Vault`: the facade the key service uses. One selected profile, at most one unlocked session.
//!
//! The API carries no UI flow: it offers the
//! primitives (status, create, unlock with or without remembering, remembered unlock, lock, item
//! reads and writes) and leaves the order of screens to the caller.
//!
//! Managed app creation writes in the sibling `Vibe Work Vault/vault/`; the low-level fixture API still creates
//! in its supplied profile folder. Both use ShellX's internal layout: `profile.json` (mode `local`, repo
//! `default`, a random embedded-server token, the keyfile, "remember this device" as requested,
//! recovery state = ShellX's default "confirmed"), `local-vault-server/repos/default/` with
//! `keyfile.json`, and, when remembering, the device secret in the OS store. **No recovery step**:
//! no recovery words are offered; recovery is the passphrase plus the keyfile in the selected folder.
//!
//! Depends on: `layout`, `managed`, `repo`, `items`, `remember`, `shellx_profile`, `chain`, vault-core. Called by:
//! vw-keys. Invariants: every method is blocking; the master key lives only in the session
//! (`Arc<MasterKey>`, zeroed when the last clone drops). [`Vault::lock`] (the user's Lock) deletes the kept device
//! key, so no remembered unlock happens until the next passphrase unlock; [`Vault::close`] (a quit that keeps the
//! last state) drops the session and keeps it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use vault_core::MasterKey;
use vault_core::keys::KdfParams;

use crate::chain::ChainGuard;
use crate::error::VaultError;
use crate::items::{self, VaultItem};
use crate::layout::{Detected, LocalVault, detect};
use crate::location::VaultLocation;
use crate::managed::{AvailableVault, ManagedVault};
use crate::remember::DeviceSecretStore;
use crate::repo::LocalRepo;

#[path = "kept_key.rs"]
mod kept_key;
#[path = "vault_unlock.rs"]
mod unlock;
#[path = "unlock_policy.rs"]
mod unlock_policy;
use unlock_policy::{keyfile_from_json, unlock_keyfile};

/// Which app layout a vault uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultKind {
    /// ShellX's shared layout.
    Shellx,
    /// The standalone Vault app's layout (opened, never created here; no remembered device).
    Desktop,
    /// Vibe Work's private folder, using the same internal layout as ShellX.
    VibeWork,
}

/// What the profile folder offers right now. Never contains a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VaultStatus {
    /// No vault yet: `create` is possible.
    NoVault,
    /// A vault that needs unlocking, or a selected vault that must be restored at its path.
    Locked {
        /// Its layout.
        kind: VaultKind,
        /// A remembered device is set up (a remembered unlock may work).
        remembered_device: bool,
    },
    /// Unlocked.
    Unlocked {
        /// Its layout.
        kind: VaultKind,
        /// This device is remembered.
        remembered_device: bool,
    },
    /// Multiple local vaults exist without a selection: `unlock` needs an exact choice.
    TwoVaults,
    /// The saved selection is retained, but its folder is not currently present.
    SelectedVaultMissing,
    /// A server vault (not supported in v0.1).
    RemoteNotSupported,
    /// Only ShellX's old profile folder exists.
    LegacyProfile,
    /// A ShellX setup was never confirmed.
    SetupUnfinished,
}

/// Construction inputs.
pub struct VaultConfig {
    /// The profile folder (checked against the canonical guard in [`Vault::new`]).
    pub location: VaultLocation,
    /// Where remembered-device secrets go.
    pub device_store: Arc<dyn DeviceSecretStore>,
    /// Argon2 settings for a new vault (`KdfParams::default()` = Vault's own).
    pub kdf: KdfParams,
    /// Device name written into manifests.
    pub device_name: String,
}

struct Session {
    kind: VaultKind,
    repo: LocalRepo,
    master: Arc<MasterKey>,
    remembered: bool,
}

/// The vault client for one profile folder.
pub struct Vault {
    config: VaultConfig,
    managed: Option<ManagedVault>,
    session: Mutex<Option<Session>>,
    manual_lock: AtomicBool,
    remember_note: Mutex<Option<String>>,
    /// the head guard every session's reads go through (kept across lock and unlock).
    chain: Arc<ChainGuard>,
}

fn locked_status(entry: &AvailableVault) -> VaultStatus {
    let remembered_device = match &entry.vault {
        LocalVault::Shellx { profile, .. } => {
            profile.remember_device && profile.remembered_keyfile_json.is_some()
        }
        LocalVault::Desktop { .. } => false,
    };
    VaultStatus::Locked {
        kind: entry.kind,
        remembered_device,
    }
}

impl Vault {
    /// A client for `config.location`.
    ///
    /// # Errors
    /// [`VaultError::CanonicalForbidden`] when the location is the forbidden real profile.
    pub fn new(mut config: VaultConfig) -> Result<Self, VaultError> {
        config.location = config.location.checked()?;
        Ok(Self {
            config,
            managed: None,
            session: Mutex::new(None),
            manual_lock: AtomicBool::new(false),
            remember_note: Mutex::new(None),
            chain: Arc::default(),
        })
    }

    /// A product vault: private creation uses a sibling container outside `data_dir`.
    /// An existing `data_dir/vault` is relocated by a verified move before selection changes.
    ///
    /// # Errors
    /// The canonical guard, path checks, migration verification or selection I/O can fail.
    pub fn new_managed(
        config: VaultConfig,
        data_dir: std::path::PathBuf,
    ) -> Result<Self, VaultError> {
        let mut vault = Self::new(config)?;
        let managed = ManagedVault::new(data_dir)?;
        managed.validate_paths()?;
        VaultLocation::explicit(&managed.private_dir).checked()?;
        managed.prepare(&vault.config.location)?;
        vault.managed = Some(managed);
        Ok(vault)
    }

    /// Existing local kinds, whether or not one has been selected.
    ///
    /// # Errors
    /// A profile or selection read can fail.
    pub fn available_vaults(&self) -> Result<Vec<VaultKind>, VaultError> {
        match &self.managed {
            Some(managed) => Ok(managed
                .available(&self.config.location)?
                .iter()
                .map(|entry| entry.kind)
                .collect()),
            None => Ok(match detect(&self.config.location)? {
                Detected::One(LocalVault::Shellx { .. }) => vec![VaultKind::Shellx],
                Detected::One(LocalVault::Desktop { .. }) => vec![VaultKind::Desktop],
                Detected::Two { .. } => vec![VaultKind::Shellx, VaultKind::Desktop],
                _ => Vec::new(),
            }),
        }
    }

    /// The persisted product selection; fixtures without managed mode report their sole local vault.
    ///
    /// # Errors
    /// The selection record is malformed or unreadable.
    pub fn selected_vault(&self) -> Result<Option<VaultKind>, VaultError> {
        match &self.managed {
            Some(managed) => managed.selected(&self.config.location),
            None => Ok(match detect(&self.config.location)? {
                Detected::One(LocalVault::Shellx { .. }) => Some(VaultKind::Shellx),
                Detected::One(LocalVault::Desktop { .. }) => Some(VaultKind::Desktop),
                _ => None,
            }),
        }
    }

    /// The selected vault's profile folder, or none before choice.
    ///
    /// # Errors
    /// The selection record is malformed or unreadable.
    pub fn selected_path(&self) -> Result<Option<std::path::PathBuf>, VaultError> {
        Ok(self.selected_vault()?.map(|kind| match &self.managed {
            Some(managed) => managed.path_for(&self.config.location, kind),
            None => self.config.location.profile_dir.clone(),
        }))
    }

    /// Migrates a pre-selection install that already wrote `keys.json` and has one shared vault.
    /// A fresh install never takes this path, so its choice screen remains available.
    ///
    /// # Errors
    /// Profile or selection I/O errors.
    pub fn select_legacy_if_used(&self) -> Result<(), VaultError> {
        let Some(managed) = &self.managed else {
            return Ok(());
        };
        if managed.has_record() {
            return Ok(());
        }
        let available = managed.available(&self.config.location)?;
        if available.len() == 1 && available[0].kind != VaultKind::VibeWork {
            managed.save(&self.config.location, available[0].kind)?;
        }
        Ok(())
    }

    /// The configured shared profile folder. In managed mode, [`Vault::selected_path`] gives
    /// the app's active profile after selection.
    #[must_use]
    pub fn location(&self) -> &VaultLocation {
        &self.config.location
    }

    /// The computer's credential store this client keeps device secrets in.
    #[must_use]
    pub fn device_store(&self) -> Arc<dyn DeviceSecretStore> {
        Arc::clone(&self.config.device_store)
    }

    fn session(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn note(&self, note: Option<String>) {
        *self
            .remember_note
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = note;
    }

    /// Why the last attempt to remember this device failed, if it did (e.g. no Secret Service).
    #[must_use]
    pub fn remember_note(&self) -> Option<String> {
        self.remember_note
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Current status.
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] / [`VaultError::Io`] from reading the profile folder.
    pub fn status(&self) -> Result<VaultStatus, VaultError> {
        if let Some(session) = self.session().as_ref() {
            return Ok(VaultStatus::Unlocked {
                kind: session.kind,
                remembered_device: session.remembered,
            });
        }
        if let Some(managed) = &self.managed {
            if managed.selected_missing(&self.config.location)? {
                return Ok(VaultStatus::SelectedVaultMissing);
            }
            let available = managed.available(&self.config.location)?;
            let selected = managed.selected(&self.config.location)?;
            if let Some(kind) = selected {
                return Ok(available.iter().find(|entry| entry.kind == kind).map_or(
                    VaultStatus::Locked {
                        kind,
                        remembered_device: false,
                    },
                    locked_status,
                ));
            }
            return Ok(match available.as_slice() {
                [] => VaultStatus::NoVault,
                [one] => locked_status(one),
                _ => VaultStatus::TwoVaults,
            });
        }
        Ok(match detect(&self.config.location)? {
            Detected::None => VaultStatus::NoVault,
            Detected::LegacyOnly => VaultStatus::LegacyProfile,
            Detected::SetupUnfinished => VaultStatus::SetupUnfinished,
            Detected::Remote => VaultStatus::RemoteNotSupported,
            Detected::Two { .. } => VaultStatus::TwoVaults,
            Detected::One(LocalVault::Shellx { profile, .. }) => VaultStatus::Locked {
                kind: VaultKind::Shellx,
                remembered_device: profile.remember_device
                    && profile.remembered_keyfile_json.is_some(),
            },
            Detected::One(LocalVault::Desktop { .. }) => VaultStatus::Locked {
                kind: VaultKind::Desktop,
                remembered_device: false,
            },
        })
    }

    /// Locks: drops the session (the master key is zeroed when the last user drops it) and deletes
    /// the kept device key, so the vault stays locked, also after a restart, until the next
    /// passphrase unlock. When the credential store cannot delete its entry, the profile no
    /// longer holds the keyfile that entry opens, and [`Vault::remember_note`] says why.
    pub fn lock(&self) {
        self.manual_lock.store(true, Ordering::SeqCst);
        *self.session() = None;
        if let Err(error) = self.forget_kept_key() {
            self.note(Some(error.to_string()));
        }
    }

    /// Drops the session and keeps the kept device key: the next start opens the vault with it
    /// (a quit that keeps the last state).
    pub fn close(&self) {
        *self.session() = None;
    }

    fn with_session<T>(
        &self,
        f: impl FnOnce(&LocalRepo, &MasterKey, &str) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let (repo, master) = {
            let guard = self.session();
            let session = guard.as_ref().ok_or(VaultError::Locked)?;
            // every item read and write checks the head against the newest one seen (`chain`).
            (
                session.repo.clone().guarded(Arc::clone(&self.chain)),
                Arc::clone(&session.master),
            )
        };
        f(&repo, &master, &self.config.device_name)
    }

    /// One item.
    ///
    /// # Errors
    /// [`VaultError::Locked`], storage errors.
    pub fn read_item(&self, id: &str) -> Result<Option<VaultItem>, VaultError> {
        self.with_session(|repo, master, _| items::read_item(repo, master, id))
    }

    /// Every readable item (secrets included: callers keep them in memory only).
    ///
    /// # Errors
    /// [`VaultError::Locked`], storage errors.
    pub fn list_items(&self) -> Result<Vec<VaultItem>, VaultError> {
        self.with_session(|repo, master, _| items::list_items(repo, master))
    }

    /// Every manifest-backed item, refusing any missing, corrupt or undecodable record. Use for authorization.
    ///
    /// # Errors
    /// [`VaultError::Locked`], [`VaultError::Corrupt`], storage errors.
    pub fn list_items_strict(&self) -> Result<Vec<VaultItem>, VaultError> {
        self.with_session(|repo, master, _| items::list_items_strict(repo, master))
    }

    /// Creates or replaces an item.
    ///
    /// # Errors
    /// [`VaultError::Locked`], [`VaultError::Conflict`], storage errors.
    pub fn upsert_item(&self, item: &VaultItem) -> Result<u64, VaultError> {
        self.with_session(|repo, master, device| items::upsert_item(repo, master, device, item))
    }

    /// an atomic read-modify-write of one item ([`items::update_item`]).
    ///
    /// # Errors
    /// As [`Vault::upsert_item`].
    pub fn update_item(
        &self,
        id: &str,
        change: impl Fn(Option<VaultItem>) -> Option<VaultItem>,
    ) -> Result<Option<VaultItem>, VaultError> {
        self.with_session(|repo, master, device| {
            items::update_item(repo, master, device, id, change)
        })
    }

    /// Deletes an item.
    ///
    /// # Errors
    /// As [`Vault::upsert_item`].
    pub fn delete_item(&self, id: &str) -> Result<u64, VaultError> {
        self.with_session(|repo, master, device| items::delete_item(repo, master, device, id))
    }

    /// Atomically validates and deletes one item in the same sealed head transaction.
    ///
    /// # Errors
    /// As [`Vault::upsert_item`].
    pub fn delete_item_when(
        &self,
        id: &str,
        accept: impl Fn(&VaultItem) -> bool,
    ) -> Result<Option<bool>, VaultError> {
        self.with_session(|repo, master, device| {
            items::delete_item_when(repo, master, device, id, accept)
        })
    }
    /// Checks and writes a set of items against one sealed head (value and permission together).
    /// # Errors
    /// Locked vault, corrupt storage or a repeatedly moving head.
    #[cfg(feature = "agent-vault")]
    pub fn update_items_checked(
        &self,
        change: impl Fn(&[VaultItem]) -> Option<Vec<VaultItem>>,
    ) -> Result<bool, VaultError> {
        self.with_session(|repo, master, device| {
            items::update_items_checked(repo, master, device, change)
        })
    }
}
