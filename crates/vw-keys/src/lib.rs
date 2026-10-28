//! The key service: the Mistral API key lives in the user's Vault and in
//! this process's memory only; `vibe-app-server` gets a placeholder and talks to Mistral through the
//! local model proxy.
//!
//! Modules: `service` (the [`KeyService`] owner), `secrets` (project secrets), `mcp` (MCP server key bindings), `logins` (website logins and their fill grants), `speech` (its Voxtral calls), `work` (Libraries and image generation), `vibe_env` (the `VIBE_*` overrides that point Vibe
//! at the proxy), `existing` (finding and importing a key Vibe itself stored), `settings` (which
//! vault item holds the key), `vault_settings` (the lock-at-quit switch), `error`.
//!
//! Depends on: vw-vault, vw-keyproxy, vw-mistral. Called by: vw-core-api (commands),
//! vw-host (construction). Invariant: client-facing commands never return a key; the proxy and scanner read values only in Rust.

mod chrome;
mod entry_defaults;
mod entry_permissions;
mod error;
pub mod existing;
mod logins;
mod mcp;
mod network;
mod scan_secrets;
mod secrets;
mod service;
mod settings;
mod speech;
mod vault_settings;
mod vibe_env;
mod work;

pub use error::KeyError;
pub use existing::{ExistingKey, ExistingKeys, ExistingSource};
pub use logins::{LoginCounts, StoredLogin};
pub use network::NetworkWait;
pub use scan_secrets::VaultScanSecret;
pub use secrets::{Stored, is_locked};
pub use service::{
    KeyEvent, KeyMetadata, KeyPresence, KeyService, KeyServiceConfig, KeySource, KeyStatus,
    MAX_KEY_LEN, MistralKey, OcrFailure, StoredKeyTest,
};
pub use speech::SpeechFailure;
pub use vibe_env::vibe_env;
pub use vw_vault::entry_permission::EntryPermission;
pub use vw_vault::fill_grant::FillGrant;
pub use vw_vault::login::Login;
pub use vw_vault::mcp_binding::McpBinding;
pub use vw_vault::project_secret::{ProjectSecret, SecretLabel};
pub use work::WorkFailure;
