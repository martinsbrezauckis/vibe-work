//! Vault creation, passphrase unlock and remembered-key unlock.
//! Called through Vault's existing API; uses the same profile, credential store and session state.
//! Selection, password checks, manual locking and key retention follow the existing order.
use super::{Session, Vault, VaultKind, keyfile_from_json, unlock_keyfile};
use crate::error::VaultError;
use crate::layout::{Detected, LocalVault, detect};
use crate::remember::{account_for, new_device_secret, remembered_keyfile_json};
use crate::repo::LocalRepo;
use crate::secret::SecretString;
use crate::shellx_profile::{RecoveryState, ShellxMode, ShellxProfile};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use vault_core::Keyfile;

impl Vault {
    /// Creates a new private Vibe Work vault in managed mode, or a ShellX-layout fixture in
    /// low-level mode, and leaves it unlocked. When remembering fails
    /// (no OS store), the vault is still created, not remembered, and [`Vault::remember_note`]
    /// says why.
    ///
    /// # Errors
    /// [`VaultError::EmptyPassphrase`], [`VaultError::AlreadyExists`],
    /// [`VaultError::SelectionMismatch`] when already selected, or profile I/O errors.
    pub fn create(&self, passphrase: &SecretString, remember: bool) -> Result<(), VaultError> {
        if let Some(managed) = &self.managed {
            if managed.present_selection(&self.config.location)?.is_some() {
                return Err(VaultError::SelectionMismatch);
            }
            if managed
                .available(&self.config.location)?
                .iter()
                .any(|entry| entry.kind == VaultKind::VibeWork)
            {
                return Err(VaultError::AlreadyExists);
            }
            let private = &managed.private_dir;
            let Some(_creating) = crate::managed::Creating::begin(private) else {
                return Err(VaultError::AlreadyExists);
            };
            self.create_at(private, VaultKind::VibeWork, passphrase, remember)?;
            if let Err(error) = managed.save_choice(&self.config.location, VaultKind::VibeWork) {
                self.lock();
                return Err(error);
            }
            return Ok(());
        }
        if passphrase.is_empty() {
            return Err(VaultError::EmptyPassphrase);
        }
        match detect(&self.config.location)? {
            Detected::None => {}
            Detected::One(_) | Detected::Two { .. } => return Err(VaultError::AlreadyExists),
            Detected::Remote => return Err(VaultError::RemoteNotSupported),
            Detected::LegacyOnly => return Err(VaultError::LegacyProfile),
            Detected::SetupUnfinished => return Err(VaultError::SetupUnfinished),
        }
        self.create_at(
            &self.config.location.profile_dir,
            VaultKind::Shellx,
            passphrase,
            remember,
        )
    }

    pub(super) fn create_at(
        &self,
        dir: &std::path::Path,
        kind: VaultKind,
        passphrase: &SecretString,
        remember: bool,
    ) -> Result<(), VaultError> {
        if passphrase.is_empty() {
            return Err(VaultError::EmptyPassphrase);
        }
        let (master, keyfile) = Keyfile::create(passphrase.expose(), self.config.kdf.clone())
            .map_err(|e| VaultError::Corrupt(e.to_string()))?;
        let keyfile_json = serde_json::to_string_pretty(&keyfile)
            .map_err(|e| VaultError::Corrupt(e.to_string()))?;
        let mut profile = ShellxProfile {
            mode: ShellxMode::Local,
            server_url: None,
            repo: "default".to_owned(),
            token: Some(hex::encode(vault_core::random_bytes::<24>())),
            keyfile_json: Some(keyfile_json.clone()),
            remember_device: false,
            remembered_keyfile_json: None,
            recovery: RecoveryState::default(),
            extra: serde_json::Map::new(),
        };
        let repo = LocalRepo::new(profile.local_repo_dir(dir));
        repo.init()?;
        repo.publish_keyfile(keyfile_json.as_bytes())?;
        self.note(None);
        if remember && let Err(error) = self.remember_into(&mut profile, &keyfile, passphrase, dir)
        {
            self.note(Some(error.to_string()));
        }
        profile.write(dir)?;
        self.manual_lock.store(false, Ordering::SeqCst);
        *self.session() = Some(Session {
            kind,
            repo,
            master: Arc::new(master),
            remembered: profile.remember_device,
        });
        Ok(())
    }

    pub(super) fn remember_into(
        &self,
        profile: &mut ShellxProfile,
        keyfile: &Keyfile,
        passphrase: &SecretString,
        dir: &std::path::Path,
    ) -> Result<(), VaultError> {
        let secret = new_device_secret();
        let json = remembered_keyfile_json(keyfile, passphrase, &secret)?;
        let account = self.device_account(dir);
        self.config.device_store.set(&account, &secret)?;
        profile.remember_device = true;
        profile.remembered_keyfile_json = Some(json);
        Ok(())
    }

    pub(super) fn forget_into(
        &self,
        profile: &mut ShellxProfile,
        dir: &std::path::Path,
    ) -> Result<(), VaultError> {
        let account = self.device_account(dir);
        self.config.device_store.delete(&account)?;
        profile.remember_device = false;
        profile.remembered_keyfile_json = None;
        Ok(())
    }

    pub(super) fn device_account(&self, dir: &std::path::Path) -> String {
        // Keep the original logical account identity after moving the encrypted profile. No OS secret
        // needs to be read, transferred or deleted as part of migration.
        let account_path = self
            .managed
            .as_ref()
            .filter(|m| m.private_dir == dir)
            .map_or(dir, |m| m.legacy_dir.as_path());
        account_for(account_path)
    }

    pub(super) fn pick(&self, choice: Option<VaultKind>) -> Result<LocalVault, VaultError> {
        if let Some(managed) = &self.managed {
            let selected = if choice.is_some() {
                managed.present_selection(&self.config.location)?
            } else {
                // An absent selection is never resolved by its kind alone: the folder that kind names now (for
                // example a private vault created later) is not the vault the user chose. Only an explicit choice
                // may open another vault.
                if managed.selected_missing(&self.config.location)? {
                    return Err(VaultError::SelectedVaultMissing);
                }
                managed.selected(&self.config.location)?
            };
            if let (Some(selected), Some(choice)) = (selected, choice)
                && selected != choice
            {
                return Err(VaultError::SelectionMismatch);
            }
            let kind = selected.or(choice).ok_or(VaultError::SelectionRequired)?;
            let available = managed.available(&self.config.location)?;
            return available
                .into_iter()
                .find(|entry| entry.kind == kind)
                .map(|entry| entry.vault)
                .ok_or_else(|| {
                    if selected.is_some() {
                        VaultError::SelectedVaultMissing
                    } else {
                        VaultError::NoVault
                    }
                });
        }
        match (detect(&self.config.location)?, choice) {
            (
                Detected::One(LocalVault::Shellx { .. }),
                Some(VaultKind::Desktop | VaultKind::VibeWork),
            )
            | (
                Detected::One(LocalVault::Desktop { .. }),
                Some(VaultKind::Shellx | VaultKind::VibeWork),
            ) => Err(VaultError::SelectionMismatch),
            (Detected::One(one), _) => Ok(one),
            (Detected::Two { shellx, .. }, Some(VaultKind::Shellx)) => Ok(shellx),
            (Detected::Two { desktop, .. }, Some(VaultKind::Desktop)) => Ok(desktop),
            (Detected::Two { .. }, Some(VaultKind::VibeWork)) | (Detected::None, _) => {
                Err(VaultError::NoVault)
            }
            (Detected::Two { .. }, None) => Err(VaultError::TwoVaults),
            (Detected::Remote, _) => Err(VaultError::RemoteNotSupported),
            (Detected::LegacyOnly, _) => Err(VaultError::LegacyProfile),
            (Detected::SetupUnfinished, _) => Err(VaultError::SetupUnfinished),
        }
    }

    /// Unlocks with the passphrase. `remember`: `Some` turns "remember this device" on or off
    /// (ShellX and Vibe Work layouts), `None` keeps the profile's setting. An unselected managed
    /// install requires `choice`, and the successful choice persists for later launches. A later
    /// different choice is refused. Remembering failure leaves an unlocked vault with a note.
    ///
    /// # Errors
    /// [`VaultError::WrongPassphrase`], selection errors, profile I/O errors.
    pub fn unlock(
        &self,
        passphrase: &SecretString,
        remember: Option<bool>,
        choice: Option<VaultKind>,
    ) -> Result<(), VaultError> {
        if passphrase.is_empty() {
            return Err(VaultError::EmptyPassphrase);
        }
        self.note(None);
        let selected_kind = self
            .managed
            .as_ref()
            .map(|managed| managed.present_selection(&self.config.location))
            .transpose()?
            .flatten();
        let chosen_kind = selected_kind.or(choice);
        let profile_dir = self.managed.as_ref().map_or_else(
            || self.config.location.profile_dir.clone(),
            |managed| {
                managed.path_for(
                    &self.config.location,
                    chosen_kind.unwrap_or(VaultKind::Shellx),
                )
            },
        );
        let session = match self.pick(choice)? {
            LocalVault::Desktop {
                keyfile_path,
                repo_dir,
            } => {
                let raw = std::fs::read_to_string(&keyfile_path)
                    .map_err(|e| VaultError::io("read the keyfile", &keyfile_path, e))?;
                let master = unlock_keyfile(&keyfile_from_json(&raw)?, passphrase)?;
                if remember == Some(true) {
                    self.note(Some(format!(
                        "the vault from {} has no remembered device",
                        vw_vocabulary::label("vaultKinds", "desktop")
                    )));
                }
                Session {
                    kind: VaultKind::Desktop,
                    repo: LocalRepo::new(repo_dir),
                    master: Arc::new(master),
                    remembered: false,
                }
            }
            LocalVault::Shellx { profile, repo_dir } => {
                let mut profile = *profile;
                let keyfile =
                    keyfile_from_json(profile.keyfile_json.as_deref().ok_or_else(|| {
                        VaultError::Corrupt("the profile has no keyfile".to_owned())
                    })?)?;
                let master = unlock_keyfile(&keyfile, passphrase)?;
                let result = if remember.unwrap_or(profile.remember_device) {
                    self.remember_into(&mut profile, &keyfile, passphrase, &profile_dir)
                } else {
                    self.forget_into(&mut profile, &profile_dir)
                };
                if let Err(error) = result {
                    profile.remember_device = false;
                    profile.remembered_keyfile_json = None;
                    self.note(Some(error.to_string()));
                }
                profile.write(&profile_dir)?;
                Session {
                    kind: chosen_kind.unwrap_or(VaultKind::Shellx),
                    repo: LocalRepo::new(repo_dir),
                    master: Arc::new(master),
                    remembered: profile.remember_device,
                }
            }
        };
        if let (Some(managed), Some(kind)) = (&self.managed, chosen_kind)
            && selected_kind.is_none()
        {
            managed.save_choice(&self.config.location, kind)?;
        }
        self.manual_lock.store(false, Ordering::SeqCst);
        *self.session() = Some(session);
        Ok(())
    }

    /// Unlocks with the remembered device when possible. `Ok(false)`: not possible (not set up,
    /// no secret in the OS store, two vaults, locked by the user since the last passphrase unlock).
    ///
    /// # Errors
    /// [`VaultError::Remember`] when the OS store fails or the stored secret no longer opens the
    /// remembered keyfile.
    pub fn unlock_remembered(&self) -> Result<bool, VaultError> {
        self.unlock_remembered_inner(false)
    }

    /// Uses a remembered device key after an explicit user unlock action, including after a manual lock.
    ///
    /// # Errors
    /// [`VaultError::Remember`] when the device store fails or its saved key cannot open the vault.
    pub fn unlock_remembered_for_user(&self) -> Result<bool, VaultError> {
        self.unlock_remembered_inner(true)
    }

    pub(super) fn unlock_remembered_inner(&self, user_requested: bool) -> Result<bool, VaultError> {
        if self.session().is_some() {
            return Ok(true);
        }
        if !user_requested && self.manual_lock.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let (profile, repo_dir, kind, profile_dir) = if let Some(managed) = &self.managed {
            if managed.selected_missing(&self.config.location)? {
                return Err(VaultError::SelectedVaultMissing);
            }
            let Some(record) = managed.read_record(&self.config.location)? else {
                return Ok(false);
            };
            let entry = managed
                .available(&self.config.location)?
                .into_iter()
                .find(|entry| entry.kind == record.kind && entry.path == record.path)
                .ok_or(VaultError::SelectedVaultMissing)?;
            let LocalVault::Shellx { profile, repo_dir } = entry.vault else {
                return Ok(false);
            };
            (profile, repo_dir, record.kind, entry.path)
        } else {
            let Detected::One(LocalVault::Shellx { profile, repo_dir }) =
                detect(&self.config.location)?
            else {
                return Ok(false);
            };
            (
                profile,
                repo_dir,
                VaultKind::Shellx,
                self.config.location.profile_dir.clone(),
            )
        };
        let Some(json) = profile
            .remembered_keyfile_json
            .as_deref()
            .filter(|_| profile.remember_device && profile.recovery.confirmed)
        else {
            return Ok(false);
        };
        let account = self.device_account(&profile_dir);
        let Some(secret) = self.config.device_store.get(&account)? else {
            return Ok(false);
        };
        let master = unlock_keyfile(&keyfile_from_json(json)?, &secret).map_err(|_| {
            VaultError::Remember("the remembered device no longer opens the vault".to_owned())
        })?;
        *self.session() = Some(Session {
            kind,
            repo: LocalRepo::new(repo_dir),
            master: Arc::new(master),
            remembered: true,
        });
        if user_requested {
            self.manual_lock.store(false, Ordering::SeqCst);
        }
        Ok(true)
    }
}
