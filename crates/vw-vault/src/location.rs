//! Where the Vault profile folder is, with the same rules as `vault-broker::profile`
//! (`third_party/shellx-vault/crates/vault-broker/src/profile.rs:85-161` @ the pin), and the guard
//! that keeps tests and development runs away from the user's real profile.
//!
//! Shared-profile discovery rules (shared with ShellX and the standalone Vault app):
//! - `SHELLX_VAULT_PROFILE_DIR` (non-empty) wins;
//! - Windows `%APPDATA%\shellx-vault`; macOS `~/Library/Application Support/shellx-vault`; Linux
//!   `$XDG_CONFIG_HOME/shellx-vault`, else `~/.config/shellx-vault`;
//! - ShellX's old location `~/.shellx/shellx-vault` is reported separately (never used).
//!
//! Managed Vibe Work creation uses a separate sibling container (`private_location`), outside
//! the bundle identifier's app-data folder and the installer's generic data deletion.
//!
//! The path is used exactly as resolved (no canonicalization): ShellX derives the remembered-device
//! keyring account from this string (`remember::account_for`).
//!
//! Depends on: std. Called by: `vault`, vw-keys. Invariant: once [`forbid_canonical_profile`] was
//! called, [`VaultLocation::checked`] refuses the canonical folder for the rest of the process, both
//! when asked for it by name and when a caller passes the same path explicitly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::VaultError;

/// The override variable shared with ShellX and the standalone Vault app.
pub const PROFILE_DIR_ENV: &str = "SHELLX_VAULT_PROFILE_DIR";
/// The folder name under the platform config folder.
pub const PROFILE_DIR_NAME: &str = "shellx-vault";

static CANONICAL_FORBIDDEN: AtomicBool = AtomicBool::new(false);

/// Forbids the user's real profile for the rest of this process. Tests and development runs call
/// it first thing; it cannot be undone.
pub fn forbid_canonical_profile() {
    CANONICAL_FORBIDDEN.store(true, Ordering::SeqCst);
}

/// True once [`forbid_canonical_profile`] ran.
#[must_use]
pub fn canonical_forbidden() -> bool {
    CANONICAL_FORBIDDEN.load(Ordering::SeqCst)
}

/// Operating system whose rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Linux and other Unix.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

impl Platform {
    /// The platform this binary runs on.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

/// Inputs of the resolution (environment values, or test values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileInput {
    /// Rules to apply.
    pub platform: Platform,
    /// `HOME`, else `USERPROFILE`.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<PathBuf>,
    /// `APPDATA`.
    pub appdata: Option<PathBuf>,
    /// `SHELLX_VAULT_PROFILE_DIR`.
    pub override_dir: Option<PathBuf>,
}

impl ProfileInput {
    /// Reads this process's environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            platform: Platform::current(),
            home: std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            appdata: std::env::var_os("APPDATA").map(PathBuf::from),
            override_dir: std::env::var_os(PROFILE_DIR_ENV).map(PathBuf::from),
        }
    }
}

/// Resolved folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDirs {
    /// The profile folder all apps share.
    pub canonical: PathBuf,
    /// ShellX's old folder (`~/.shellx/shellx-vault`), only reported.
    pub legacy: PathBuf,
    /// True when `SHELLX_VAULT_PROFILE_DIR` chose `canonical`.
    pub from_override: bool,
}

fn non_empty(path: Option<&PathBuf>) -> Option<&PathBuf> {
    path.filter(|p| !p.as_os_str().is_empty())
}

/// Resolves the folders the way `vault-broker::profile::resolve_profile_dirs` does.
///
/// # Errors
/// [`VaultError::NoVault`] is never returned here; a missing `HOME` (or `APPDATA` on Windows)
/// gives [`VaultError::Corrupt`] with the variable's name, as upstream refuses too.
pub fn resolve(input: &ProfileInput) -> Result<ProfileDirs, VaultError> {
    let home = non_empty(input.home.as_ref())
        .ok_or_else(|| VaultError::Corrupt("HOME/USERPROFILE not set".to_owned()))?;
    let legacy = home.join(".shellx").join(PROFILE_DIR_NAME);
    if let Some(dir) = non_empty(input.override_dir.as_ref()) {
        return Ok(ProfileDirs {
            canonical: dir.clone(),
            legacy,
            from_override: true,
        });
    }
    let base = match input.platform {
        Platform::Windows => non_empty(input.appdata.as_ref())
            .cloned()
            .ok_or_else(|| VaultError::Corrupt("APPDATA not set".to_owned()))?,
        Platform::Macos => home.join("Library/Application Support"),
        Platform::Linux => non_empty(input.xdg_config_home.as_ref())
            .cloned()
            .unwrap_or_else(|| home.join(".config")),
    };
    Ok(ProfileDirs {
        canonical: base.join(PROFILE_DIR_NAME),
        legacy,
        from_override: false,
    })
}

/// The profile folder a [`crate::Vault`] works in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultLocation {
    /// The profile folder.
    pub profile_dir: PathBuf,
    /// ShellX's old folder (reported as [`VaultError::LegacyProfile`] when only it exists).
    pub legacy_dir: PathBuf,
}

impl VaultLocation {
    /// The user's real profile, from this process's environment.
    ///
    /// # Errors
    /// [`VaultError::CanonicalForbidden`] after [`forbid_canonical_profile`]; resolution errors.
    pub fn canonical() -> Result<Self, VaultError> {
        if canonical_forbidden() {
            return Err(VaultError::CanonicalForbidden);
        }
        let dirs = resolve(&ProfileInput::from_env())?;
        Ok(Self {
            profile_dir: dirs.canonical,
            legacy_dir: dirs.legacy,
        })
    }

    /// An explicit folder (tests, development runs, a run folder). `legacy_dir` defaults to a
    /// sibling that does not exist.
    #[must_use]
    pub fn explicit(profile_dir: impl Into<PathBuf>) -> Self {
        let profile_dir = profile_dir.into();
        let legacy_dir = profile_dir.join(".no-legacy-profile");
        Self {
            profile_dir,
            legacy_dir,
        }
    }

    /// Returns `self` unless the canonical profile is forbidden and this location is it (compared
    /// with the path the environment resolves to, and with the default path ignoring any
    /// override, so a test cannot reach the real folder by setting the override to it either).
    ///
    /// # Errors
    /// [`VaultError::CanonicalForbidden`].
    pub fn checked(self) -> Result<Self, VaultError> {
        if !canonical_forbidden() {
            return Ok(self);
        }
        let mut input = ProfileInput::from_env();
        let mut real = Vec::new();
        if let Ok(dirs) = resolve(&input) {
            real.push(dirs.canonical);
        }
        input.override_dir = None;
        if let Ok(dirs) = resolve(&input) {
            real.push(dirs.canonical);
            real.push(dirs.legacy);
        }
        if real.iter().any(|path| same_path(path, &self.profile_dir)) {
            return Err(VaultError::CanonicalForbidden);
        }
        Ok(self)
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
