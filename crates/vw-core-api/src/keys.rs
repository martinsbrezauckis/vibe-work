//! Key and vault commands: the one-way commands the setup screen, Settings, and Debug API call.
//! They go to the core's [`KeyService`]; none reaches
//! Vibe and none returns a secret:
//! - inputs that are secret (passphrase, API key) are [`SecretInput`]: redacted in `Debug`, written
//!   as `"<redacted>"` if a command is ever serialized, zeroed on drop;
//! - results are statuses, counts and metadata; `setupStoreKey`, `removeKey`, `importVibeKey`,
//!   `removeVibeKeyOriginal`, `createVault`, `unlockVault`, `lockVault` and `useExistingKey`
//!   answer `done` (ok) or an error, nothing else.
//!
//! Who may change the key and the vault: the window only (on the vault the Debug API has
//! the agent's rights; `vault_scope` refuses it every op here except the reads `readStatus`, `listKeyCandidates`,
//! `detectVibeKey` and `testKey`). What never changes: no
//! command, result, event or log carries the key or the passphrase as a value (the Debug API's response
//! guard in vw-debug-api also refuses to send any registered secret).
//!
//! Depends on: vw-keys, vw-vault, `error`. Called by: `dispatch`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use vw_keys::{ExistingSource, KeyError, KeyPresence, KeyService, KeySource, KeyStatus};
use vw_vault::{SecretString, VaultError, VaultKind, VaultStatus};

use crate::command::CommandResult;
use crate::error::{CoreError, ErrorCode};

/// A secret command input (passphrase or API key).
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct SecretInput(SecretString);

impl SecretInput {
    /// Wraps a value (tests and adapters).
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretString::new(value))
    }

    /// The wrapped secret (a project secret's value on its way into the vault).
    pub(crate) fn into_secret(self) -> SecretString {
        self.0
    }

    /// True when the value starts with `prefix` (the private test switch accepts only test-prefixed
    /// throwaway values; private test builds only).
    #[cfg(feature = "vault-e2e")]
    pub(crate) fn has_prefix(&self, prefix: &str) -> bool {
        self.0.expose().starts_with(prefix)
    }
}

impl std::fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Serialize for SecretInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("<redacted>")
    }
}

impl<'de> Deserialize<'de> for SecretInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = zeroize::Zeroizing::new(String::deserialize(deserializer)?);
        Ok(Self(SecretString::new(raw.as_str())))
    }
}

/// Which local vault to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum VaultChoice {
    /// ShellX's layout.
    Shellx,
    /// The standalone Vault app's layout.
    Desktop,
    /// Vibe Work's private vault.
    VibeWork,
}

/// Where Vibe's own copy of a key is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum VibeKeySource {
    /// `~/.vibe/.env`.
    DotEnv,
    /// Vibe's keyring entry.
    Keyring,
}

impl From<VibeKeySource> for ExistingSource {
    fn from(source: VibeKeySource) -> Self {
        match source {
            VibeKeySource::DotEnv => Self::DotEnv,
            VibeKeySource::Keyring => Self::Keyring,
        }
    }
}

/// One key or vault operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum KeyRequest {
    /// Vault and key status (no secret).
    ReadStatus {},
    /// Creates a new private Vibe Work vault and unlocks it in a managed app.
    CreateVault {
        /// The new passphrase.
        passphrase: SecretInput,
        /// Remember this device (product default: on).
        remember_device: bool,
    },
    /// Unlocks with the passphrase.
    UnlockVault {
        /// The passphrase.
        passphrase: SecretInput,
        /// Turn "remember this device" on or off (absent: keep).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        remember_device: Option<bool>,
        /// Which vault, when two exist.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        vault: Option<VaultChoice>,
    },
    /// Locks the vault (model requests wait for the next unlock).
    LockVault {},
    /// Stores the key (first setup or replacement). One-way: answers `done` or an error.
    /// Replacing an active key asks the native confirmation before storage.
    SetupStoreKey {
        /// The key.
        api_key: SecretInput,
    },
    /// Stops using the key (deletes Vibe Work's own vault resource; never another app's item).
    RemoveKey {},
    /// Mistral credentials already in the vault (metadata).
    ListKeyCandidates {},
    /// Uses one of them instead of storing a copy.
    UseExistingKey {
        /// Its vault item id.
        item_id: String,
    },
    /// `GET /v1/models` with the key in use (nothing is spent).
    TestKey {},
    /// Where Vibe itself holds a key (no values).
    DetectVibeKey {},
    /// Moves Vibe's own copy into the vault (the original stays).
    ImportVibeKey {
        /// From where.
        source: VibeKeySource,
    },
    /// Deletes Vibe's own copy (only after the user agreed).
    RemoveVibeKeyOriginal {
        /// Which one.
        source: VibeKeySource,
    },
}

/// Key and vault status for the UI. No secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KeyStatusView {
    /// `noVault`, `locked`, `unlocked`, `twoVaults`, `remoteNotSupported`, `legacyProfile`,
    /// `setupUnfinished`.
    pub vault_state: String,
    /// `vibeWork`, `shellx` or `desktop` when locked or unlocked.
    pub vault_kind: Option<String>,
    /// Actual existing local vaults; an unselected install can offer each for use.
    pub available_vaults: Vec<VaultChoice>,
    /// A managed install has persisted a vault choice.
    pub vault_selected: bool,
    /// The selected vault's profile folder, or null before a choice.
    pub vault_path: Option<String>,
    /// This device is (or can be) unlocked without the passphrase.
    pub remembered_device: bool,
    /// `unknown` (locked), `missing`, `ready`.
    pub key_state: String,
    /// The vault item the key comes from, when ready.
    pub key_item_id: Option<String>,
    /// True when that item is Vibe Work's own resource.
    pub own_key: bool,
    /// The active vault item's nonsecret details; null when locked, missing or unavailable.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub active_key: Option<KeyMetadataView>,
    /// Why remembering this device did not work, if it did not.
    pub remember_note: Option<String>,
}

/// Provenance known for the active key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum KeySourceView {
    /// Selected from the vault or stored directly here.
    Vault,
    /// Imported from Vibe's own credential location.
    Vibe,
}

/// Nonsecret details of the active Mistral key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KeyMetadataView {
    /// Vault display title.
    pub label: String,
    /// Known origin of this item.
    pub source: KeySourceView,
    /// Original creation time in Unix milliseconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub added_ms: i64,
}

/// The per-minute rate-limit headroom Mistral reported in its latest answer through the model proxy
/// One answer's facts: a field that answer did not carry is `null`. Numbers only, no
/// secret. Mistral exposes no plan or monthly quota, so none is here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RateLimitsView {
    /// Tokens per minute the key may use.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub tokens_limit: Option<u64>,
    /// Tokens left in the current minute.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub tokens_remaining: Option<u64>,
    /// Requests per minute.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub requests_limit: Option<u64>,
    /// Requests left in the current minute.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub requests_remaining: Option<u64>,
    /// When that answer arrived, milliseconds since the Unix epoch. The numbers are per minute: after
    /// about 60 s without a newer answer they are history, not the current headroom.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub observed_at_ms: i64,
}

impl From<vw_keyproxy::RateLimits> for RateLimitsView {
    fn from(l: vw_keyproxy::RateLimits) -> Self {
        Self {
            tokens_limit: l.tokens_limit,
            tokens_remaining: l.tokens_remaining,
            requests_limit: l.requests_limit,
            requests_remaining: l.requests_remaining,
            observed_at_ms: l.observed_at_ms,
        }
    }
}

fn kind_name(kind: VaultKind) -> String {
    match kind {
        VaultKind::Shellx => "shellx",
        VaultKind::Desktop => "desktop",
        VaultKind::VibeWork => "vibeWork",
    }
    .to_owned()
}

impl From<KeyStatus> for KeyStatusView {
    fn from(status: KeyStatus) -> Self {
        let available_vaults = status
            .available_vaults
            .iter()
            .map(|kind| match kind {
                VaultKind::Shellx => VaultChoice::Shellx,
                VaultKind::Desktop => VaultChoice::Desktop,
                VaultKind::VibeWork => VaultChoice::VibeWork,
            })
            .collect();
        let vault_selected = status.vault_selected;
        let vault_path = status
            .vault_path
            .map(|path| path.to_string_lossy().into_owned());
        let (vault_state, vault_kind, remembered_device) = match status.vault {
            VaultStatus::NoVault => ("noVault", None, false),
            VaultStatus::Locked {
                kind,
                remembered_device,
            } => ("locked", Some(kind_name(kind)), remembered_device),
            VaultStatus::Unlocked {
                kind,
                remembered_device,
            } => ("unlocked", Some(kind_name(kind)), remembered_device),
            VaultStatus::TwoVaults => ("twoVaults", None, false),
            VaultStatus::RemoteNotSupported => ("remoteNotSupported", None, false),
            VaultStatus::LegacyProfile => ("legacyProfile", None, false),
            VaultStatus::SetupUnfinished => ("setupUnfinished", None, false),
        };
        let (key_state, key_item_id, own_key) = match status.key {
            KeyPresence::Unknown => ("unknown", None, false),
            KeyPresence::Missing => ("missing", None, false),
            KeyPresence::Ready { item_id, own } => ("ready", Some(item_id), own),
        };
        Self {
            vault_state: vault_state.to_owned(),
            vault_kind,
            available_vaults,
            vault_selected,
            vault_path,
            remembered_device,
            key_state: key_state.to_owned(),
            key_item_id,
            own_key,
            active_key: status.active_key.map(|item| KeyMetadataView {
                label: item.label,
                source: match item.source {
                    KeySource::Vault => KeySourceView::Vault,
                    KeySource::Vibe => KeySourceView::Vibe,
                },
                added_ms: item.added_ms,
            }),
            remember_note: status.remember_note,
        }
    }
}

/// A Mistral credential already in the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KeyCandidateView {
    /// Vault item id.
    pub item_id: String,
    /// Its title.
    pub label: String,
    /// `resource` or `compat`.
    pub source: String,
}

/// A place where Vibe itself holds a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VibeKeyView {
    /// Where.
    pub source: VibeKeySource,
    /// File path or keyring service.
    pub location: String,
    /// True for Vibe's keychain item on macOS (setup names the keychain when it offers the removal).
    pub mac_keychain: bool,
}

/// A stable reason word for a key-service error (in `CoreError::server_code`).
pub(crate) fn reason(error: &KeyError) -> &'static str {
    match error {
        KeyError::Vault(v) => match v {
            VaultError::CanonicalForbidden => "canonicalForbidden",
            VaultError::NoVault => "noVault",
            VaultError::TwoVaults => "twoVaults",
            VaultError::SelectionRequired => "selectionRequired",
            VaultError::SelectionMismatch => "selectionMismatch",
            VaultError::SelectedVaultMissing => "selectedVaultMissing",
            VaultError::RemoteNotSupported => "remoteNotSupported",
            VaultError::LegacyProfile => "legacyProfile",
            VaultError::SetupUnfinished => "setupUnfinished",
            VaultError::AlreadyExists => "alreadyExists",
            VaultError::Locked => "locked",
            VaultError::WrongPassphrase => "wrongPassphrase",
            VaultError::EmptyPassphrase => "emptyPassphrase",
            VaultError::Remember(_) => "rememberFailed",
            VaultError::Conflict { .. } => "conflict",
            VaultError::Corrupt(_) => "corrupt",
            VaultError::Io { .. } => "io",
        },
        KeyError::NoKey => "noKey",
        KeyError::InvalidKey => "invalidKey",
        KeyError::PermissionsUnreadable => "permissionsUnreadable",
        KeyError::Test(t) => match t {
            vw_mistral::TestCallError::Unauthorized => "unauthorized",
            vw_mistral::TestCallError::Forbidden => "forbidden",
            vw_mistral::TestCallError::RateLimited => "rateLimited",
            vw_mistral::TestCallError::Server(_) => "mistralServerError",
            vw_mistral::TestCallError::Unexpected(_) => "unexpectedStatus",
            vw_mistral::TestCallError::Network(_) => "network",
            vw_mistral::TestCallError::MalformedKey => "invalidKey",
        },
        KeyError::NotACandidate => "notACandidate",
        KeyError::NothingToImport => "nothingToImport",
        KeyError::Existing(_) => "existingKeyChange",
        KeyError::Settings(_) => "settings",
        KeyError::InvalidSecret(_) => "invalidSecret",
        KeyError::NameTaken => "nameTaken",
        KeyError::NotOwnSecret => "notOwnSecret",
        KeyError::NoSuchSecret => "noSuchSecret",
        KeyError::Internal => "internal",
    }
}

pub(crate) fn key_error(error: &KeyError) -> CoreError {
    let mut core = CoreError::new(ErrorCode::KeyService, error.to_string());
    core.server_code = Some(reason(error).to_owned());
    core
}

/// Executes one key request on `keys` (module header). Every origin runs it directly.
///
/// # Errors
/// [`ErrorCode::KeyService`] with a stable reason word in `server_code`.
pub(crate) async fn execute(
    keys: &KeyService,
    request: KeyRequest,
) -> Result<CommandResult, CoreError> {
    let done = Ok(CommandResult::Done {});
    let map = |e: KeyError| key_error(&e);
    match request {
        KeyRequest::ReadStatus {} => Ok(CommandResult::KeyStatus {
            status: keys.status().await.map_err(map)?.into(),
        }),
        KeyRequest::CreateVault {
            passphrase,
            remember_device,
        } => {
            // Registered only once the vault holds it (an unconfirmed input stays out of the redaction list).
            let kept = passphrase.0.clone();
            keys.create_vault(passphrase.0, remember_device)
                .await
                .map_err(map)?;
            kept.register_as(vw_secrets::SecretKind::Typed);
            done
        }
        KeyRequest::UnlockVault {
            passphrase,
            remember_device,
            vault,
        } => {
            let choice = vault.map(|v| match v {
                VaultChoice::Shellx => VaultKind::Shellx,
                VaultChoice::Desktop => VaultKind::Desktop,
                VaultChoice::VibeWork => VaultKind::VibeWork,
            });
            let kept = passphrase.0.clone();
            keys.unlock(passphrase.0, remember_device, choice)
                .await
                .map_err(map)?;
            kept.register_as(vw_secrets::SecretKind::Typed);
            done
        }
        KeyRequest::LockVault {} => {
            keys.lock();
            done
        }
        KeyRequest::SetupStoreKey { api_key } => {
            let kept = api_key.0.clone();
            keys.store_key(api_key.0).await.map_err(map)?;
            kept.register_as(vw_secrets::SecretKind::MistralKey);
            done
        }
        KeyRequest::RemoveKey {} => keys.remove_key().await.map_err(map).and(done),
        KeyRequest::ListKeyCandidates {} => Ok(CommandResult::KeyCandidates {
            candidates: keys
                .candidates()
                .await
                .map_err(map)?
                .into_iter()
                .map(|c| KeyCandidateView {
                    item_id: c.item_id,
                    label: c.label,
                    source: c.source.to_owned(),
                })
                .collect(),
        }),
        KeyRequest::UseExistingKey { item_id } => {
            keys.use_existing(item_id).await.map_err(map).and(done)
        }
        KeyRequest::TestKey {} => Ok(CommandResult::KeyTest {
            model_count: u32::try_from(keys.test_key().await.map_err(map)?.model_count)
                .unwrap_or(u32::MAX),
        }),
        KeyRequest::DetectVibeKey {} => Ok(CommandResult::VibeKeys {
            found: keys
                .detect_existing()
                .await
                .map_err(map)?
                .into_iter()
                .map(|k| VibeKeyView {
                    source: match k.source {
                        ExistingSource::DotEnv => VibeKeySource::DotEnv,
                        ExistingSource::Keyring => VibeKeySource::Keyring,
                    },
                    location: k.location,
                    mac_keychain: k.mac_keychain,
                })
                .collect(),
        }),
        KeyRequest::ImportVibeKey { source } => keys
            .import_existing(source.into())
            .await
            .map_err(map)
            .and(done),
        KeyRequest::RemoveVibeKeyOriginal { source } => keys
            .remove_existing_original(source.into())
            .await
            .map_err(map)
            .and(done),
    }
}
