//! Which vault the profile folder holds. Two app-layer layouts can share one folder and the same storage core:
//! - **ShellX** (`shellx_profile`): `profile.json` with a `mode` key; local repository in
//!   `local-vault-server/repos/<repo>`.
//! - **Vault app** (`crates/tauri-plugin-vault/src/profile.rs:27-41,192-250` @ the pin):
//!   `local-keyfile.json`, local repository in `local-store/repos/default`, a `profile.json`
//!   (`snake_case` `server`/`repo`/`token`/`keyfile_json`, no `mode`) only for a remote vault.
//!
//! Detection only reads; when both local vaults exist the result says so and nothing is merged.
//!
//! Depends on: `shellx_profile`, `location`. Called by: `vault`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::VaultError;
use crate::location::VaultLocation;
use crate::shellx_profile::{PROFILE_FILE, ShellxMode, ShellxProfile};

/// The Vault app's keyfile for its local vault.
pub const DESKTOP_KEYFILE: &str = "local-keyfile.json";
/// The Vault app's local data folder.
pub const DESKTOP_DATA_DIR: &str = "local-store";
/// ShellX's old profile file name, in its old folder (`vault-broker/src/profile.rs:8`).
pub const LEGACY_PROFILE_FILE: &str = "shellx-profile.json";

/// A local vault this client can open.
#[derive(Debug, Clone)]
pub enum LocalVault {
    /// A ShellX-layout vault.
    Shellx {
        /// Its profile.
        profile: Box<ShellxProfile>,
        /// Its repository folder.
        repo_dir: PathBuf,
    },
    /// A standalone Vault app vault.
    Desktop {
        /// `local-keyfile.json`.
        keyfile_path: PathBuf,
        /// Its repository folder.
        repo_dir: PathBuf,
    },
}

/// What the profile folder holds.
#[derive(Debug, Clone)]
pub enum Detected {
    /// Nothing usable yet (a new vault may be created).
    None,
    /// ShellX's old folder only.
    LegacyOnly,
    /// A ShellX setup that was never confirmed.
    SetupUnfinished,
    /// A vault on a server (not supported in v0.1).
    Remote,
    /// One local vault.
    One(LocalVault),
    /// A ShellX vault and a Vault app vault.
    Two {
        /// The ShellX one.
        shellx: LocalVault,
        /// The Vault app one.
        desktop: LocalVault,
    },
}

/// Inspects the profile folder (read-only).
///
/// # Errors
/// [`VaultError::Corrupt`] for a `profile.json` that is neither layout's, [`VaultError::Io`].
pub fn detect(location: &VaultLocation) -> Result<Detected, VaultError> {
    let dir = &location.profile_dir;
    let raw_profile = read_json(&dir.join(PROFILE_FILE))?;
    let desktop = desktop_local(dir);
    let mut remote = false;
    let mut shellx = None;
    let mut unfinished = false;
    if let Some(value) = raw_profile {
        if value.get("mode").is_some() {
            let profile = ShellxProfile::read(dir)?
                .ok_or_else(|| VaultError::Corrupt("profile.json vanished".to_owned()))?;
            match profile.mode {
                ShellxMode::Local if !profile.recovery.confirmed => unfinished = true,
                ShellxMode::Local => {
                    let repo_dir = profile.local_repo_dir(dir);
                    shellx = Some(LocalVault::Shellx {
                        profile: Box::new(profile),
                        repo_dir,
                    });
                }
                ShellxMode::External => remote = true,
                ShellxMode::Unconfigured => {}
            }
        } else if value
            .get("server")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            // The Vault app's "remembered connection" to a server.
            remote = true;
        }
    }
    Ok(match (shellx, desktop) {
        (Some(shellx), Some(desktop)) => Detected::Two { shellx, desktop },
        (Some(one), None) | (None, Some(one)) => Detected::One(one),
        (None, None) if unfinished => Detected::SetupUnfinished,
        (None, None) if remote => Detected::Remote,
        (None, None) if location.legacy_dir.join(LEGACY_PROFILE_FILE).is_file() => {
            Detected::LegacyOnly
        }
        (None, None) => Detected::None,
    })
}

fn desktop_local(dir: &Path) -> Option<LocalVault> {
    let keyfile_path = dir.join(DESKTOP_KEYFILE);
    keyfile_path.is_file().then(|| LocalVault::Desktop {
        keyfile_path,
        repo_dir: dir.join(DESKTOP_DATA_DIR).join("repos").join("default"),
    })
}

fn read_json(path: &Path) -> Result<Option<Value>, VaultError> {
    match std::fs::read(path) {
        Ok(raw) => serde_json::from_slice(&raw)
            .map(Some)
            .map_err(|_| VaultError::Corrupt("profile.json is not JSON".to_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(VaultError::io("read the vault profile", path, e)),
    }
}
