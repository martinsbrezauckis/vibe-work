//! Errors of the key service. Texts are for people; none contains a key, passphrase or token.
//!
//! Depends on: vw-vault, vw-mistral. Called by: `service`, vw-core-api (maps them to `CoreError`).

use vw_mistral::TestCallError;
use vw_vault::VaultError;

/// Why a key operation failed.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// From the vault (locked, wrong passphrase, two vaults,...).
    #[error(transparent)]
    Vault(#[from] VaultError),
    /// No key is stored or chosen yet.
    #[error("no Mistral API key is set up yet")]
    NoKey,
    /// The typed key cannot be a Mistral key (empty, spaces, control characters, too long).
    #[error("that does not look like an API key (empty, contains spaces or is too long)")]
    InvalidKey,
    /// The test call failed.
    #[error(transparent)]
    Test(#[from] TestCallError),
    /// The chosen vault item is not a Mistral credential.
    #[error("that vault item does not hold a Mistral key")]
    NotACandidate,
    /// Nothing was found at the import source.
    #[error("no Mistral key was found there")]
    NothingToImport,
    /// Reading or deleting Vibe's own copy failed.
    #[error("Vibe's own key copy could not be changed: {0}")]
    Existing(String),
    /// The key service's settings file could not be written.
    #[error("could not save the key setting: {0}")]
    Settings(String),
    /// A project secret's name or value is not acceptable.
    #[error("{0}")]
    InvalidSecret(&'static str),
    /// Another app's secret already has this name.
    #[error("a secret of another app already has this name; choose another name")]
    NameTaken,
    /// The secret was not created by Vibe Work, so Vibe Work does not remove it.
    #[error("this secret belongs to another app or to you in Vault; remove it there")]
    NotOwnSecret,
    /// No secret an agent may use has this id (absent, user-only, or empty).
    #[error("no such secret (it may have been removed, or it is for you only)")]
    NoSuchSecret,
    /// A worker thread failed (panic in a blocking task).
    #[error("internal error in the key service")]
    Internal,
}
