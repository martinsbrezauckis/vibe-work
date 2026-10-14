//! Typed errors of the Vault client. Messages are written for people (shows some of them as
//! is) and never contain a secret value, a passphrase or key material.
//!
//! Depends on: thiserror. Called by: every module of this crate and its callers.

use std::path::PathBuf;

/// Why a vault operation failed.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The process has forbidden the user's real Vault profile (tests, development runs) and a
    /// caller tried to use it anyway.
    #[error(
        "the real Vault profile is off limits in this run (tests and development runs use their own folder)"
    )]
    CanonicalForbidden,
    /// No vault exists in the profile folder yet.
    #[error("there is no vault on this computer yet")]
    NoVault,
    /// Both a ShellX vault and a standalone Vault app vault exist; the caller must name one.
    #[error(
        "there are two vaults on this computer ({shellx} and {desktop}); choose which one to use",
        shellx = vw_vocabulary::label("vaultKinds", "shellx"),
        desktop = vw_vocabulary::label("vaultKinds", "desktop")
    )]
    TwoVaults,
    /// A choice is required before the app can use an existing vault.
    #[error("choose which vault to use before unlocking")]
    SelectionRequired,
    /// The selected vault cannot be replaced by another kind without an explicit migration flow.
    #[error("the selected vault cannot be switched; the selected vault must be restored")]
    SelectionMismatch,
    /// A persisted selection points to a vault that is no longer present.
    #[error("the selected vault is missing; restore it at its original location")]
    SelectedVaultMissing,
    /// The profile points at a vault server; v0.1 supports only vaults on this computer.
    #[error(
        "this vault is kept on a server; Vibe Work supports only vaults on this computer for now"
    )]
    RemoteNotSupported,
    /// Only the old ShellX profile location exists; ShellX moves it on its next start.
    #[error("the vault profile is in ShellX's old location; open ShellX once so it moves it")]
    LegacyProfile,
    /// ShellX started a setup that was never confirmed; ShellX itself refuses to unlock it.
    #[error("vault setup was not finished in ShellX; finish it there first")]
    SetupUnfinished,
    /// `create` was called although a vault exists.
    #[error("a vault already exists on this computer")]
    AlreadyExists,
    /// The vault is locked (no passphrase or remembered device used yet).
    #[error("the vault is locked")]
    Locked,
    /// The passphrase (or remembered-device secret) does not open the keyfile.
    #[error("wrong passphrase")]
    WrongPassphrase,
    /// An empty passphrase was given.
    #[error("the passphrase is empty")]
    EmptyPassphrase,
    /// The OS credential store failed (no Secret Service session, access denied,...).
    #[error("this device cannot be remembered: {0}")]
    Remember(String),
    /// Another app kept changing the vault while this one tried to save; nothing was overwritten.
    #[error(
        "another app changed the vault while saving; nothing was overwritten (tried {attempts} times)"
    )]
    Conflict {
        /// Attempts made.
        attempts: u32,
    },
    /// Stored data failed authentication or does not parse.
    #[error("vault data is damaged: {0}")]
    Corrupt(String),
    /// A file operation failed.
    #[error("{context} ({path}): {source}")]
    Io {
        /// What was being done.
        context: &'static str,
        /// The file or folder.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
}

impl VaultError {
    /// Wraps an I/O error with what was being done and where.
    pub(crate) fn io(
        context: &'static str,
        path: &std::path::Path,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            context,
            path: path.to_path_buf(),
            source,
        }
    }
}
