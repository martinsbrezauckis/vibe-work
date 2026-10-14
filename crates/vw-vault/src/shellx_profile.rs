//! ShellX's `profile.json` (the app layer of a ShellX vault, also used for new vaults), as
//! written by ShellX @ `e38c2b5c` `src-tauri/src/shellx_vault/backend.rs:83-102` (struct),
//! `:174-184` (defaults), `:965-982` (`write_profile`: adds a top-level `server` key) and
//! `recovery.rs:12-27` (`RecoveryState`, default `confirmed: true`).
//!
//! A local ShellX vault keeps its repository at `<profile>/local-vault-server/repos/<repo>`
//! (`backend.rs:1196-1231`), its keyfile in `keyfileJson` (also published to the repository), a random
//! embedded-server token in `token`, and, with "remember this device", `rememberedKeyfileJson` (the
//! keyfile rewrapped under a device secret held in the OS credential store, see `remember`).
//!
//! Depends on: serde, `repo` (private writes). Called by: `layout`, `vault`.
//! Invariants: fields this code does not know are kept on rewrite (`extra`); the file is replaced
//! atomically and user-only.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::VaultError;
use crate::repo::{create_private_dir_all, write_synced};

/// File name inside the profile folder (`vault-broker/src/profile.rs:7`).
pub const PROFILE_FILE: &str = "profile.json";
/// Data folder of ShellX's embedded local server.
pub const LOCAL_DATA_DIR: &str = "local-vault-server";

/// ShellX vault mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ShellxMode {
    /// Not set up.
    Unconfigured,
    /// Repository on this computer.
    Local,
    /// Repository on a vault server.
    External,
}

fn default_mode() -> ShellxMode {
    ShellxMode::External
}

fn default_repo() -> String {
    "default".to_owned()
}

fn default_true() -> bool {
    true
}

/// ShellX's recovery confirmation state. Vibe Work offers no recovery-word step (recovery is the
/// passphrase plus the keyfile), so it writes the same default ShellX uses for profiles without
/// that step: confirmed, no time, nothing pending.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryState {
    /// ShellX refuses to unlock when false.
    pub confirmed: bool,
    /// When it was confirmed.
    pub confirmed_at_ms: Option<i64>,
    /// A setup waiting for confirmation.
    pub pending_confirmation_id: Option<String>,
}

impl Default for RecoveryState {
    fn default() -> Self {
        Self {
            confirmed: true,
            confirmed_at_ms: None,
            pending_confirmation_id: None,
        }
    }
}

/// ShellX's `VaultProfile`, plus every unknown field.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShellxProfile {
    /// Mode (missing = external, as in ShellX).
    #[serde(default = "default_mode")]
    pub mode: ShellxMode,
    /// Server URL (external) or the last embedded-server URL (local, unused on unlock).
    #[serde(default)]
    pub server_url: Option<String>,
    /// Repository name.
    #[serde(default = "default_repo")]
    pub repo: String,
    /// Server token (local: the embedded server's token).
    #[serde(default)]
    pub token: Option<String>,
    /// The passphrase-wrapped keyfile.
    #[serde(default)]
    pub keyfile_json: Option<String>,
    /// Remember this device (ShellX default true).
    #[serde(default = "default_true")]
    pub remember_device: bool,
    /// The keyfile rewrapped under the device secret.
    #[serde(default)]
    pub remembered_keyfile_json: Option<String>,
    /// Recovery confirmation.
    #[serde(default)]
    pub recovery: RecoveryState,
    /// Fields ShellX (or a newer app) wrote that this code does not know, incl. `server`.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl std::fmt::Debug for ShellxProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellxProfile")
            .field("mode", &self.mode)
            .field("repo", &self.repo)
            .field("remember_device", &self.remember_device)
            .finish_non_exhaustive()
    }
}

impl ShellxProfile {
    /// Reads `profile.json` from `profile_dir` (`None` when absent).
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] when it is not a ShellX profile, [`VaultError::Io`].
    pub fn read(profile_dir: &Path) -> Result<Option<Self>, VaultError> {
        let path = profile_dir.join(PROFILE_FILE);
        let raw = match std::fs::read(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(VaultError::io("read the vault profile", &path, e)),
        };
        serde_json::from_slice(&raw).map(Some).map_err(|_| {
            VaultError::Corrupt("profile.json is not a ShellX vault profile".to_owned())
        })
    }

    /// The repository folder of a local vault.
    #[must_use]
    pub fn local_repo_dir(&self, profile_dir: &Path) -> PathBuf {
        profile_dir
            .join(LOCAL_DATA_DIR)
            .join("repos")
            .join(&self.repo)
    }

    /// Writes the profile atomically and user-only, with ShellX's extra top-level `server` key
    /// (`backend.rs:968-975`: the server URL when known; Vibe Work writes an empty string when it is
    /// not, so the standalone Vault app's `profile.json` parser, which needs a string, still parses).
    ///
    /// # Errors
    /// [`VaultError::Io`], [`VaultError::Corrupt`].
    pub fn write(&self, profile_dir: &Path) -> Result<(), VaultError> {
        create_private_dir_all(profile_dir)?;
        let mut value =
            serde_json::to_value(self).map_err(|e| VaultError::Corrupt(e.to_string()))?;
        if let Some(object) = value.as_object_mut()
            && !object.contains_key("server")
        {
            let server = self.server_url.clone().unwrap_or_default();
            object.insert("server".to_owned(), Value::String(server));
        }
        let bytes =
            serde_json::to_vec_pretty(&value).map_err(|e| VaultError::Corrupt(e.to_string()))?;
        let path = profile_dir.join(PROFILE_FILE);
        let tmp = profile_dir.join(format!(
            ".profile-{}.tmp",
            hex::encode(vault_core::random_bytes::<8>())
        ));
        write_synced(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            VaultError::io("replace the vault profile", &path, e)
        })
    }
}
