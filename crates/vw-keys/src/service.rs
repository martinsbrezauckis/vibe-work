//! `KeyService`: the one owner of the Mistral key in the core. It ties the vault (where
//! the key is stored), the model proxy (the only reader of the key at run time) and the supervisor
//! (which gets the placeholder and the `VIBE_*` overrides, never the key).
//!
//! Flow: the vault's last state (its kept key, or the lock-at-quit switch) and network-record verification precede
//! proxy construction; the key, once readable, goes
//! into the proxy's key cell; a locked vault or a missing key makes requests wait and raises a
//! [`KeyEvent::KeyNeeded`] (shows the unlock or setup prompt; a turn never fails silently).
//! Storing, replacing or removing the key updates the cell at once (no Vibe restart).
//!
//! Every operation is one-way: none returns the key. Blocking work (Argon2, files, OS keyring) runs
//! on `spawn_blocking`.
//!
//! Depends on: vw-vault, vw-keyproxy, vw-mistral, `existing`, `vibe_env`, `settings`, `last_state`. Called by:
//! vw-core-api (commands), vw-host (construction, launch config).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, PoisonError, Weak};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use vw_keyproxy::{
    KeyCell, KeyNeed, KeyState, ProxyConfig, ProxyHandle, ProxySecretProvider, RateLimits,
    TokenSlot,
};
use vw_mistral::{TestCallError, TestCallOk};
use vw_vault::mistral_key::{self, Candidate};
use vw_vault::{SecretString, Vault, VaultConfig, VaultKind, VaultStatus};
use zeroize::Zeroizing;

#[path = "last_state.rs"]
mod last_state;
#[path = "mistral_keys.rs"]
mod mistral_keys;
pub use mistral_keys::{MistralKey, StoredKeyTest};

use crate::error::KeyError;
use crate::existing::{ExistingKey, ExistingKeys, ExistingSource};
use crate::settings::KeySettings;
use crate::vault_settings::VaultSettings;

/// Longest key the service accepts (Mistral keys are 32 characters).
pub const MAX_KEY_LEN: usize = 256;

/// Construction inputs.
pub struct KeyServiceConfig {
    /// The vault client's settings.
    pub vault: VaultConfig,
    /// App data folder for managed selection; the private vault lives in a separate sibling.
    /// `None` keeps the
    /// low-level shared-profile fixture behavior.
    pub managed_vault: Option<PathBuf>,
    /// The proxy's upstreams and wait limit (`on_wait` is set by the service).
    pub proxy: ProxyConfig,
    /// Where the test call goes (`https://api.mistral.ai`; tests: a fake).
    pub test_api_base: String,
    /// The service's settings file (which vault item holds the key; no secret).
    pub settings_path: PathBuf,
    /// Where Vibe's own copy of a key may be.
    pub existing: ExistingKeys,
}

/// Something the UI should know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyEvent {
    /// A model request waits for a key: unlock or set up.
    KeyNeeded(KeyNeed),
    /// The status changed (unlock, lock, key stored or removed).
    StatusChanged,
    /// The key in use was replaced (stored, imported, another item chosen). Vibe remembers a
    /// credential the provider refused per session runtime, keyed by the placeholder it sends
    /// (`app_server/_provider_credentials.py:120-132`, 2.26.0); after a 401 the same session would
    /// never try again even with a good key behind the proxy. The core reopens the sessions so they
    /// start with a fresh runtime (`vw-core-api` `recovery::reopen_after_key_change`).
    KeyChanged,
    /// An answer from Mistral reported the per-minute rate-limit headroom; the latest
    /// snapshot is also kept ([`KeyService::rate_limits`]).
    RateLimits(Option<RateLimits>),
    /// A temporary Mistral source file could not be deleted after retries. Kept for later subscribers too.
    RemoteFileCleanupFailed(String),
    /// Saved network settings that waited for the vault were checked at the unlock: in use now, or refused.
    NetworkSettingsChecked(bool),
}

/// Why an OCR request did not run or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OcrFailure {
    /// No key is available: the vault is locked or no key is set up (the prompt was raised).
    #[error("{}", match .0 {
        KeyNeed::Unlock => "Unlock your vault to read documents, then choose Try again on the attachment.",
        KeyNeed::Setup => "Set up your Mistral API key to read documents.",
    })]
    KeyNeeded(KeyNeed),
    /// Mistral's OCR answered with an error or could not be reached.
    #[error(transparent)]
    Ocr(vw_mistral::OcrError),
}

/// Is a key available to the proxy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum KeyPresence {
    /// The vault is locked (or absent): unknown.
    Unknown,
    /// Unlocked, no key yet.
    Missing,
    /// A key is in use.
    Ready {
        /// The vault item it comes from.
        item_id: String,
        /// True when it is Vibe Work's own resource (false: an existing item the user chose).
        own: bool,
    },
}

/// Provenance known for the active key; older owned resources have no import marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Selected from the vault or stored directly in Vibe Work.
    Vault,
    /// Imported from Vibe's own credential location by Vibe Work.
    Vibe,
}

/// Nonsecret details of the selected vault item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyMetadata {
    /// The vault item's display title.
    pub label: String,
    /// Provenance that can be established from the item, never inferred from its ID.
    pub source: KeySource,
    /// Original item creation time in Unix milliseconds.
    pub added_ms: i64,
}

/// The service's status. No secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    /// The vault.
    pub vault: VaultStatus,
    /// Local vault layouts found on this computer.
    pub available_vaults: Vec<VaultKind>,
    /// True when a managed install has persisted a choice.
    pub vault_selected: bool,
    /// The selected profile directory, if any.
    pub vault_path: Option<PathBuf>,
    /// The key.
    pub key: KeyPresence,
    /// Details of the active key when its vault item can be read.
    pub active_key: Option<KeyMetadata>,
    /// Why remembering this device did not work, if it did not.
    pub remember_note: Option<String>,
    /// "Lock the vault when Vibe Work quits".
    pub lock_on_quit: bool,
}

/// The key service.
pub struct KeyService {
    vault: Arc<Vault>,
    pub(crate) proxy: tokio::sync::Mutex<Option<ProxyHandle>>,
    proxy_base: String,
    key: KeyCell,
    tokens: Arc<TokenSlot>,
    pub(crate) client: vw_mistral::ClientSlot,
    test_api_base: String,
    last_key_test: std::sync::Mutex<Option<KeyTest>>,
    settings: KeySettings,
    pub(crate) chrome_owner_account: String,
    vault_settings: VaultSettings,
    /// The switch as this run uses it (also when its file could not be written).
    lock_on_quit: std::sync::atomic::AtomicBool,
    /// The kept key opens nothing until a password unlock (`last_state`).
    locked_until_password: std::sync::atomic::AtomicBool,
    selection: tokio::sync::Mutex<()>,
    existing: ExistingKeys,
    pub(crate) events: broadcast::Sender<KeyEvent>,
    cleanup_notices: Arc<std::sync::Mutex<Vec<String>>>,
    /// The latest rate-limit snapshot the proxy saw; `None` until an answer carried one.
    limits: Arc<std::sync::Mutex<Option<RateLimits>>>,
    /// The proxy's MCP resolver slot, filled by the core (`set_mcp_resolver`).
    pub(crate) mcp: vw_keyproxy::McpResolverSlot,
    /// how often the logins were looked up and a password was handed out (diagnostics; never values).
    pub(crate) login_counters: crate::logins::Counters,
    pub(crate) network: std::sync::Mutex<crate::network::Network>,
}

struct KeyTest {
    // Weak identity neither keeps a removed key alive nor applies its result to a replacement.
    key: Weak<Zeroizing<String>>,
    result: Result<TestCallOk, TestCallError>,
}

pub(super) fn valid_key(key: &SecretString) -> bool {
    let k = key.expose();
    !k.is_empty() && k.len() <= MAX_KEY_LEN && k.chars().all(|c| c.is_ascii_graphic())
}

pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, KeyError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| KeyError::Internal)
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

impl KeyService {
    /// Starts the proxy and restores the vault's last state: unlocked with the kept key, or locked
    /// (the kept key deleted) when "Lock the vault when Vibe Work quits" is on.
    ///
    /// # Errors
    /// The vault location is refused ([`vw_vault::VaultError::CanonicalForbidden`]) or the proxy
    /// cannot start.
    pub async fn start(config: KeyServiceConfig) -> Result<Arc<Self>, KeyError> {
        Self::start_with_network(config, None).await
    }

    /// Starts after verifying saved network settings with the existing vault, before any key-bearing client exists.
    ///
    /// # Errors
    /// As [`Self::start`]. An unavailable vault leaves custom network settings unapplied.
    pub async fn start_with_network(
        config: KeyServiceConfig,
        network_file: Option<PathBuf>,
    ) -> Result<Arc<Self>, KeyError> {
        let vault = Arc::new(match config.managed_vault {
            Some(data_dir) => Vault::new_managed(config.vault, data_dir)?,
            None => Vault::new(config.vault)?,
        });
        if config.settings_path.is_file() {
            vault.select_legacy_if_used()?;
        }
        let vault_settings = VaultSettings::next_to(&config.settings_path);
        let (network_vault, unlock_vault) = (Arc::clone(&vault), Arc::clone(&vault));
        let (at_start, vault_settings) = blocking(move || {
            let at_start = last_state::restore(&unlock_vault, &vault_settings);
            (at_start, vault_settings)
        })
        .await?;
        let lock_on_quit = at_start.lock_on_quit;
        let network =
            blocking(move || crate::network::launch(&network_vault, network_file, lock_on_quit))
                .await?;
        let (events, _) = broadcast::channel(64);
        let tokens = Arc::new(TokenSlot::default());
        let key = KeyCell::new(KeyState::Missing(KeyNeed::Unlock));
        let hook_events = events.clone();
        let mut proxy_config = config.proxy;
        let mcp = proxy_config.mcp.clone();
        proxy_config.on_wait = Arc::new(move |need| {
            let _ = hook_events.send(KeyEvent::KeyNeeded(need));
        });
        let limits: Arc<std::sync::Mutex<Option<RateLimits>>> = Arc::default();
        let (limits_cell, limits_events) = (Arc::clone(&limits), events.clone());
        proxy_config.on_rate_limits = Arc::new(move |snapshot| {
            *limits_cell.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
            let _ = limits_events.send(KeyEvent::RateLimits(Some(snapshot)));
        });
        let proxy = ProxyHandle::start(proxy_config, Arc::clone(&tokens), key.clone())
            .await
            .map_err(|e| KeyError::Settings(e.to_string()))?;
        // No redirects: the key goes only to the host we chose (test call and OCR send it directly).
        let client =
            vw_mistral::ClientSlot::new().map_err(|e| KeyError::Settings(e.to_string()))?;
        let service = Arc::new(Self {
            vault,
            proxy_base: proxy.base_url(),
            proxy: tokio::sync::Mutex::new(Some(proxy)),
            key,
            tokens,
            client,
            test_api_base: config.test_api_base,
            last_key_test: std::sync::Mutex::default(),
            chrome_owner_account: crate::chrome::account(&config.settings_path),
            settings: KeySettings::new(config.settings_path),
            vault_settings,
            lock_on_quit: std::sync::atomic::AtomicBool::new(at_start.lock_on_quit),
            locked_until_password: std::sync::atomic::AtomicBool::new(
                at_start.locked_until_password,
            ),
            selection: tokio::sync::Mutex::new(()),
            existing: config.existing,
            events,
            cleanup_notices: Arc::default(),
            limits,
            mcp,
            login_counters: crate::logins::Counters::default(),
            network: std::sync::Mutex::new(network),
        });
        service.refresh_key().await;
        Ok(service)
    }

    /// The selected vault profile folder (a path, no secret); before selection, the configured
    /// shared folder. compares it with the isolated test profile.
    #[must_use]
    pub fn vault_profile_dir(&self) -> PathBuf {
        self.vault
            .selected_path()
            .ok()
            .flatten()
            .unwrap_or_else(|| self.vault.location().profile_dir.clone())
    }

    /// The latest per-minute rate-limit snapshot a Mistral answer reported through the proxy
    /// or `None` when none has carried one yet.
    #[must_use]
    pub fn rate_limits(&self) -> Option<RateLimits> {
        *self.limits.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The supervisor's secret provider: a new placeholder token per server start.
    #[must_use]
    pub fn secret_provider(&self) -> Arc<ProxySecretProvider> {
        Arc::new(ProxySecretProvider::new(Arc::clone(&self.tokens)))
    }

    /// The `VIBE_*` overrides for `vibe-app-server` (see `vibe_env`).
    #[must_use]
    pub fn vibe_env(&self) -> Vec<(String, OsString)> {
        crate::vibe_env::vibe_env(&self.proxy_base)
    }

    /// The proxy's base URL (`http://127.0.0.1:<port>`).
    #[must_use]
    pub fn proxy_base(&self) -> &str {
        &self.proxy_base
    }

    /// The proxy's counters (tests and diagnostics).
    pub async fn proxy_stats(&self) -> Option<Arc<vw_keyproxy::ProxyStats>> {
        self.proxy
            .lock()
            .await
            .as_ref()
            .map(|p| Arc::clone(p.stats()))
    }

    /// Events (key needed, status changed).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<KeyEvent> {
        self.events.subscribe()
    }

    /// Unconfirmed remote deletions from this run, newest last (at most 100); no key or file contents.
    #[must_use]
    pub fn remote_cleanup_notices(&self) -> Vec<String> {
        self.cleanup_notices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn cleanup_notice_handler(&self) -> vw_mistral::work::CleanupNotice {
        let events = self.events.clone();
        let notices = self.cleanup_notices.clone();
        Arc::new(move |message| {
            let mut saved = notices
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if saved.len() == 100 {
                saved.remove(0);
            }
            saved.push(message.clone());
            drop(saved);
            let _ = events.send(KeyEvent::RemoteFileCleanupFailed(message));
        })
    }

    fn changed(&self) {
        let _ = self.events.send(KeyEvent::StatusChanged);
    }

    /// The vault client (project secrets, `secrets`).
    pub(crate) fn vault(&self) -> Arc<Vault> {
        Arc::clone(&self.vault)
    }

    /// Reads the key from the vault into the proxy's cell (or marks why there is none).
    async fn refresh_key(&self) {
        let vault = Arc::clone(&self.vault);
        let item_id = self.settings.key_item_id();
        let read = blocking(move || {
            // Every vault-opening path ends here: stored values are registered for the Debug API first.
            crate::scan_secrets::open_and_registered(&vault)?;
            match vault.read_item(&item_id) {
                Ok(Some(item)) => mistral_key::key_in_item(&item).ok_or(KeyNeed::Setup),
                _ => Err(KeyNeed::Setup),
            }
        })
        .await;
        match read {
            Ok(Ok(key)) => self.key.set_key(Zeroizing::new(key.expose().to_owned())),
            Ok(Err(need)) => self.key.clear(need),
            Err(_) => self.key.clear(KeyNeed::Unlock),
        }
        self.changed();
    }

    /// A different credential starts without the previous credential's headroom.
    pub(super) fn key_changed(&self) {
        *self.limits.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let _ = self.events.send(KeyEvent::RateLimits(None));
        let _ = self.events.send(KeyEvent::KeyChanged);
    }

    /// Current status.
    ///
    /// # Errors
    /// Vault read errors.
    pub async fn status(&self) -> Result<KeyStatus, KeyError> {
        let vault = Arc::clone(&self.vault);
        let (status, available, selected, path, note) = blocking(move || {
            (
                vault.status(),
                vault.available_vaults(),
                vault.selected_vault(),
                vault.selected_path(),
                vault.remember_note(),
            )
        })
        .await?;
        let status = status?;
        let item_id = self.settings.key_item_id();
        let key = match (&status, self.key.current()) {
            (VaultStatus::Unlocked { .. }, KeyState::Ready(_)) => KeyPresence::Ready {
                own: mistral_key::is_own(&item_id),
                item_id: item_id.clone(),
            },
            (VaultStatus::Unlocked { .. }, KeyState::Missing(_)) => KeyPresence::Missing,
            _ => KeyPresence::Unknown,
        };
        let active_key = if matches!(&key, KeyPresence::Ready { .. }) {
            let vault = Arc::clone(&self.vault);
            let item_id = item_id.clone();
            blocking(move || vault.read_item(&item_id))
                .await??
                .map(|item| {
                    let imported = item.id == mistral_key::RESOURCE_ID
                        && vw_vault::items::VaultResource::from_item(&item).is_some_and(
                            |resource| {
                                resource
                                    .public_fields
                                    .get("importedFrom")
                                    .and_then(Value::as_str)
                                    == Some("vibe")
                            },
                        );
                    KeyMetadata {
                        label: item.title,
                        source: if imported {
                            KeySource::Vibe
                        } else {
                            KeySource::Vault
                        },
                        added_ms: item.created_ms,
                    }
                })
        } else {
            None
        };
        Ok(KeyStatus {
            vault: status,
            available_vaults: available?,
            vault_selected: selected?.is_some(),
            vault_path: path?,
            key,
            active_key,
            remember_note: note,
            lock_on_quit: self.lock_on_quit(),
        })
    }

    /// Creates and selects the private Vibe Work vault in managed app mode, then leaves it unlocked
    /// with its key kept in the credential store (the vault keeps its last state).
    /// Low-level fixture configurations retain ShellX-layout creation in their supplied folder.
    ///
    /// # Errors
    /// [`KeyError::Vault`].
    pub async fn create_vault(&self, passphrase: SecretString) -> Result<(), KeyError> {
        let vault = Arc::clone(&self.vault);
        blocking(move || vault.create(&passphrase, true)).await??;
        self.password_typed();
        self.network_unlocked().await;
        self.refresh_key().await;
        Ok(())
    }

    /// Unlocks with the passphrase (`choice`: which of two vaults) and keeps the key in the
    /// credential store (the Vault app's layout has no kept key).
    ///
    /// # Errors
    /// [`KeyError::Vault`].
    pub async fn unlock(
        &self,
        passphrase: SecretString,
        choice: Option<VaultKind>,
    ) -> Result<(), KeyError> {
        let vault = Arc::clone(&self.vault);
        blocking(move || vault.unlock(&passphrase, Some(true), choice)).await??;
        self.password_typed();
        self.network_unlocked().await;
        self.refresh_key().await;
        Ok(())
    }

    /// Tries the device's existing remembered unlock for an explicit user action, even after a manual lock.
    ///
    /// # Errors
    /// [`KeyError::Vault`] when the saved device key cannot unlock the vault.
    pub async fn unlock_remembered(&self) -> Result<bool, KeyError> {
        if self.waits_for_password() {
            return Ok(false);
        }
        let vault = Arc::clone(&self.vault);
        let unlocked = blocking(move || vault.unlock_remembered_for_user()).await??;
        if unlocked {
            self.network_unlocked().await;
            self.refresh_key().await;
        }
        Ok(unlocked)
    }

    /// Locks the vault and deletes its kept key; model requests wait for the next unlock, also
    /// after a restart.
    pub fn lock(&self) {
        self.vault.lock();
        self.key.clear(KeyNeed::Unlock);
        self.changed();
    }

    /// Stores (or replaces) the key as Vibe Work's own vault resource and starts using it.
    ///
    /// # Errors
    /// [`KeyError::InvalidKey`], [`KeyError::Vault`] (locked, conflict,...).
    pub async fn store_key(&self, key: SecretString) -> Result<(), KeyError> {
        self.store_key_from(key, KeySource::Vault).await
    }

    async fn store_key_from(&self, key: SecretString, source: KeySource) -> Result<(), KeyError> {
        let _selection = self.selection.lock().await;
        if !valid_key(&key) {
            return Err(KeyError::InvalidKey);
        }
        let vault = Arc::clone(&self.vault);
        let stored = key.clone();
        let selected = self.settings.key_item_id();
        let target = if mistral_key::is_own(&selected) {
            selected
        } else {
            mistral_key::RESOURCE_ID.to_owned()
        };
        let item_id = target.clone();
        blocking(move || {
            let previous = vault.read_item(&item_id)?;
            let mut resource = mistral_key::resource(
                &stored,
                previous.as_ref().map(|item| item.created_ms),
                now_ms(),
            );
            if item_id != mistral_key::RESOURCE_ID {
                resource.id = item_id;
                if let Some(item) = previous {
                    resource.label = item.title;
                }
            }
            if source == KeySource::Vibe {
                resource
                    .public_fields
                    .insert("importedFrom".to_owned(), Value::String("vibe".to_owned()));
            }
            vault.upsert_item(&resource.to_item()?)
        })
        .await??;
        self.settings.set_key_item_id(&target)?;
        self.key.set_key(Zeroizing::new(key.expose().to_owned()));
        self.changed();
        self.key_changed();
        Ok(())
    }

    /// Mistral credentials already in the vault (metadata only).
    ///
    /// # Errors
    /// [`KeyError::Vault`].
    pub async fn candidates(&self) -> Result<Vec<Candidate>, KeyError> {
        let vault = Arc::clone(&self.vault);
        Ok(mistral_key::find_existing(
            &blocking(move || vault.list_items()).await??,
        ))
    }

    /// Uses an existing vault item as the key (read from it each unlock; not copied).
    ///
    /// # Errors
    /// [`KeyError::NotACandidate`], [`KeyError::Vault`].
    pub async fn use_existing(&self, item_id: String) -> Result<(), KeyError> {
        let _selection = self.selection.lock().await;
        if !self
            .candidates()
            .await?
            .iter()
            .any(|c| c.item_id == item_id)
        {
            return Err(KeyError::NotACandidate);
        }
        self.settings.set_key_item_id(&item_id)?;
        self.refresh_key().await;
        self.key_changed();
        Ok(())
    }

    /// The test call with the key in use (`GET /v1/models` direct to Mistral).
    ///
    /// # Errors
    /// [`KeyError::NoKey`], [`KeyError::Test`].
    pub async fn test_key(&self) -> Result<TestCallOk, KeyError> {
        let KeyState::Ready(key) = self.key.current() else {
            return Err(KeyError::NoKey);
        };
        let item_id = self.settings.key_item_id();
        let result =
            vw_mistral::list_models(&self.api().0, &self.test_api_base, key.as_str()).await;
        self.record_model_test(item_id, key.as_str().to_owned(), &result)
            .await?;
        let mut latest = self
            .last_key_test
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // A test that finishes after rotation must not overwrite the new key's result.
        if matches!(self.key.current(), KeyState::Ready(current) if Arc::ptr_eq(&current, &key)) {
            *latest = Some(KeyTest {
                key: Arc::downgrade(&key),
                result: result.clone(),
            });
        }
        result.map_err(KeyError::Test)
    }

    /// Whether the active key's latest completed test answered 401 or 403.
    /// Untested keys and other failures still need replacement confirmation.
    #[must_use]
    pub fn key_was_rejected(&self) -> bool {
        let latest = self
            .last_key_test
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (Some(test), KeyState::Ready(current)) = (latest.as_ref(), self.key.current()) else {
            return false;
        };
        test.key.ptr_eq(&Arc::downgrade(&current))
            && matches!(
                test.result,
                Err(TestCallError::Unauthorized | TestCallError::Forbidden)
            )
    }

    /// Reads one document with Mistral OCR using the key in use (a direct call to
    /// `test_api_base`, like the test call, not through the model proxy). One-way: the key is never
    /// returned. Without a key it raises [`KeyEvent::KeyNeeded`] (the unlock or setup prompt) and fails
    /// with [`OcrFailure::KeyNeeded`]; nothing is retried behind the user's back.
    ///
    /// # Errors
    /// [`OcrFailure`].
    pub async fn ocr(
        &self,
        input: &vw_mistral::OcrInput<'_>,
        timeout: std::time::Duration,
    ) -> Result<vw_mistral::OcrOutput, OcrFailure> {
        let key = self.key_for_call().map_err(OcrFailure::KeyNeeded)?;
        vw_mistral::ocr(
            &self.client.get(),
            &self.test_api_base,
            key.as_str(),
            input,
            timeout,
        )
        .await
        .map_err(OcrFailure::Ocr)
    }

    /// One short chat completion with the key in use (the commit message draft), a direct call to
    /// `test_api_base` like OCR. Unlike OCR it raises no unlock or setup prompt: without a key the caller
    /// falls back to its own text. Returns `None` without a key.
    ///
    /// # Errors
    /// [`vw_mistral::chat::ChatError`] when Mistral answers with an error or cannot be reached.
    pub async fn chat(
        &self,
        request: vw_mistral::chat::ChatRequest<'_>,
    ) -> Result<Option<String>, vw_mistral::chat::ChatError> {
        let KeyState::Ready(key) = self.key.current() else {
            return Ok(None);
        };
        let (client, base) = self.api();
        vw_mistral::chat::chat(&client, base, key.as_str(), request)
            .await
            .map(Some)
    }

    /// The key for one of the core's own calls (OCR, speech). Without one it raises
    /// [`KeyEvent::KeyNeeded`] (the unlock or setup prompt) and says which; nothing waits or retries.
    /// Whether voice has a ready key, without raising an unlock/setup prompt.
    /// Used before a requested reading so a refusal leaves the window unchanged.
    #[must_use]
    pub fn speech_available(&self) -> bool {
        matches!(self.key.current(), KeyState::Ready(_))
    }

    pub(crate) fn key_for_call(&self) -> Result<Arc<Zeroizing<String>>, KeyNeed> {
        match self.key.current() {
            KeyState::Ready(key) => Ok(key),
            KeyState::Missing(need) => {
                let _ = self.events.send(KeyEvent::KeyNeeded(need));
                Err(need)
            }
        }
    }

    /// The no-redirect HTTPS client and the API base of the core's own calls (`speech`).
    pub(crate) fn api(&self) -> (reqwest::Client, &str) {
        (self.client.get(), &self.test_api_base)
    }

    /// Where Vibe itself holds a key (no values).
    ///
    /// # Errors
    /// [`KeyError::Internal`].
    pub async fn detect_existing(&self) -> Result<Vec<ExistingKey>, KeyError> {
        let existing = self.existing.clone();
        blocking(move || existing.detect()).await
    }

    /// Moves the key found at `source` into the vault (the original stays until
    /// [`KeyService::remove_existing_original`]).
    ///
    /// # Errors
    /// [`KeyError::NothingToImport`], as [`KeyService::store_key`].
    pub async fn import_existing(&self, source: ExistingSource) -> Result<(), KeyError> {
        let existing = self.existing.clone();
        let key = blocking(move || existing.read(source))
            .await?
            .ok_or(KeyError::NothingToImport)?;
        self.store_key_from(key, KeySource::Vibe).await
    }

    /// Deletes Vibe's own copy at `source` (explicit user consent in the UI).
    ///
    /// # Errors
    /// [`KeyError::Existing`].
    pub async fn remove_existing_original(&self, source: ExistingSource) -> Result<(), KeyError> {
        let existing = self.existing.clone();
        blocking(move || existing.remove(source))
            .await?
            .map_err(KeyError::Existing)
    }

    /// Stops the proxy and ends the vault session for the quit (app exit, update restart): with
    /// "Lock the vault when Vibe Work quits" on it locks (deleting the kept key), otherwise the
    /// kept key stays for the next start.
    pub async fn shutdown(&self) {
        if let Some(proxy) = self.proxy.lock().await.take() {
            proxy.shutdown().await;
        }
        if self.lock_on_quit() {
            self.vault.lock();
            crate::network::drop_copy(&self.vault);
        } else {
            self.vault.close();
        }
        self.key.clear(KeyNeed::Unlock);
    }
}
