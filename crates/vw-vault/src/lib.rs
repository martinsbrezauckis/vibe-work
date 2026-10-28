//! Vibe Work's client of the Vault storage core: it selects a local
//! vault, unlocks it (passphrase or ShellX's "remember this device" scheme), and reads and writes the
//! few resources Vibe Work owns (the Mistral API key, the project secrets and the MCP server key
//! bindings, entry permissions, and website logins and fill
//! grants).
//!
//! What is shared with Vault and what belongs to this crate:
//! - **Shared storage core, byte-identical:** the crate `vault-core` (keyfile, master key, AEAD,
//!   chunking, manifests), copied unchanged into `third_party/shellx-vault`. Everything written here
//!   uses the same format. A managed private vault lives outside the install's app-data folder;
//!   only explicitly selected shared vaults are discovered by ShellX and the standalone Vault.
//! - **Ours, mirroring the same formats:** the on-disk repository access (`repo`, the layout of
//!   `vault-server`'s store), the item and typed-resource JSON (`items`, the schema of
//!   `vault-client::items::VaultItem` and `vault-broker::resources::VaultResource`), the profile
//!   folder rules (`location`, as `vault-broker::profile`), the two app-layer layouts (`layout`), and
//!   ShellX's remembered device (`remember`). Private tests compare each of them with the copied
//!   upstream crates (`vw_vault_contract`, `vw_vault_interop`).
//!
//! Depends on: vault-core, keyring-core (+ the OS store crate), serde. Called by: vw-keys.
//! Invariants:
//! - No secret is ever logged, formatted (`Debug` redacts) or written outside the vault's own
//!   encrypted storage and the OS credential store.
//! - Writes never overwrite another app's commit: the head is re-read under a file lock and the next
//!   generation is published create-new (`repo::LocalRepo::commit_if_head`).
//! - A vault's reads never go back to an older version within one process (`chain`).
//! - Every function here is blocking (Argon2, file I/O, OS keyring): async callers use
//!   `spawn_blocking`.

pub mod chain;
pub mod entry_permission;
mod error;
pub mod fill_grant;
pub mod items;
pub mod layout;
pub mod location;
pub mod login;
mod managed;
pub mod mcp_binding;
pub mod mistral_key;
pub mod private_location;
pub mod project_secret;
pub mod remember;
pub mod repo;
mod secret;
pub mod shellx_profile;
mod vault;

pub use error::VaultError;
pub use secret::SecretString;
pub use vault::{Vault, VaultConfig, VaultKind, VaultStatus};
/// Generates a password with the vendored vault core's operating-system randomness.
pub use vault_core::password::generate_password;

/// Re-exported so callers can pass test-strength KDF settings (the default is Vault's own).
pub use vault_core::keys::KdfParams;
