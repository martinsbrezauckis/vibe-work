//! Finding a Mistral key the user already gave Vibe itself, so setup can
//! offer to move it into the vault instead of asking again (the vault then holds the only copy once the
//! user removes Vibe's):
//! - Vibe's `.env` fallback `<VIBE_HOME>/.env` (`MISTRAL_API_KEY=...`, written by python-dotenv's
//!   `set_key`, `setup/auth/api_key_persistence.py`);
//! - Vibe's keyring entry: service `ai.mistral.vibe` (legacy `vibe`), user `MISTRAL_API_KEY`
//!   (`vibe/utils/keyring.py`). Entry naming per OS as python `keyring` stores it: Linux Secret Service
//!   attributes `service`/`username`; Windows generic credential with target name = the service
//!   (`keyring/backends/Windows.py`); macOS generic password (service, account) in the login keychain.
//!
//! Detection reports where a key is, never the value. Import reads the value into the vault only.
//! Removing the original is a separate, explicit command (the user's consent).
//!
//! Depends on: keyring-core (via vw-vault's OS store), vw-vault (`SecretString`). Called by: `service`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use vw_vault::SecretString;
use vw_vault::remember::{CALL_TIMEOUT, bounded};

/// Vibe's keyring service.
pub const VIBE_KEYRING_SERVICE: &str = "ai.mistral.vibe";
/// Vibe's legacy keyring service.
pub const VIBE_LEGACY_KEYRING_SERVICE: &str = "vibe";
/// The keyring user name and the `.env` variable.
pub const VIBE_KEY_NAME: &str = "MISTRAL_API_KEY";

/// Where an existing key was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExistingSource {
    /// Vibe's `.env` file.
    DotEnv,
    /// Vibe's keyring entry (current or legacy service).
    Keyring,
}

/// A found key (no value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingKey {
    /// Where.
    pub source: ExistingSource,
    /// The file path or the keyring service name.
    pub location: String,
    /// True for Vibe's keychain item on macOS (setup names the keychain when it offers the removal).
    pub mac_keychain: bool,
}

/// Vibe's keyring entry, behind a trait so tests and SSH runs use memory.
pub trait VibeKeyring: Send + Sync {
    /// The value under `service` / [`VIBE_KEY_NAME`].
    ///
    /// # Errors
    /// The store's error text.
    fn get(&self, service: &str) -> Result<Option<SecretString>, String>;
    /// Deletes it (absent is fine).
    ///
    /// # Errors
    /// The store's error text.
    fn delete(&self, service: &str) -> Result<(), String>;
}

/// The OS credential store, entry naming as python `keyring` uses it.
pub struct OsVibeKeyring {
    store: Arc<keyring_core::api::CredentialStore>,
}

impl OsVibeKeyring {
    /// Opens the platform store.
    ///
    /// # Errors
    /// The store's error text (no Secret Service session,...).
    pub fn new() -> Result<Self, String> {
        vw_vault::remember::os_credential_store()
            .map(|store| Self { store })
            .map_err(|e| e.to_string())
    }

    fn entry(&self, service: &str) -> Result<keyring_core::Entry, String> {
        // Windows: python keyring's target name is the bare service name.
        let target: HashMap<&str, &str> = if cfg!(windows) {
            HashMap::from([("target", service)])
        } else {
            HashMap::new()
        };
        let modifiers = (!target.is_empty()).then_some(&target);
        self.store
            .build(service, VIBE_KEY_NAME, modifiers)
            .map_err(|e| e.to_string())
    }
}

// Each call has the OS store's deadline (`vw_vault::remember::bounded`): a silent keyring answers "did not answer".
impl VibeKeyring for OsVibeKeyring {
    fn get(&self, service: &str) -> Result<Option<SecretString>, String> {
        let entry = self.entry(service)?;
        let found = bounded(CALL_TIMEOUT, move || match entry.get_password() {
            Ok(value) => Ok(Some(SecretString::new(value))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        })?;
        if let Some(key) = &found {
            key.register_as(vw_secrets::SecretKind::MistralKey);
        }
        Ok(found)
    }

    fn delete(&self, service: &str) -> Result<(), String> {
        let entry = self.entry(service)?;
        bounded(CALL_TIMEOUT, move || match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        })
    }
}

/// In-memory keyring (tests, SSH runs where the OS store is unavailable).
#[derive(Default)]
pub struct MemoryVibeKeyring(pub Mutex<HashMap<String, SecretString>>);

impl VibeKeyring for MemoryVibeKeyring {
    fn get(&self, service: &str) -> Result<Option<SecretString>, String> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(service)
            .cloned())
    }

    fn delete(&self, service: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(service);
        Ok(())
    }
}

/// Where to look.
#[derive(Clone)]
pub struct ExistingKeys {
    /// Vibe's `.env` (product: `~/.vibe/.env`; tests: a fixture).
    pub dotenv: PathBuf,
    /// Vibe's keyring.
    pub keyring: Arc<dyn VibeKeyring>,
}

fn unquote(value: &str) -> &str {
    let v = value.trim();
    for quote in ['\'', '"'] {
        if let Some(inner) = v.strip_prefix(quote).and_then(|r| r.strip_suffix(quote)) {
            return inner;
        }
    }
    v.split(" #").next().unwrap_or(v).trim()
}

fn dotenv_line_key(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    if line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (name, value) = line.split_once('=')?;
    Some((name.trim(), value))
}

/// The key in a `.env` file, if present and non-empty.
#[must_use]
pub fn read_dotenv_key(path: &Path) -> Option<SecretString> {
    let text = zeroize::Zeroizing::new(std::fs::read_to_string(path).ok()?);
    text.lines()
        .filter_map(dotenv_line_key)
        .filter(|(name, _)| *name == VIBE_KEY_NAME)
        .map(|(_, value)| SecretString::new(unquote(value)))
        .rfind(|key| !key.is_empty())
        .inspect(|key| key.register_as(vw_secrets::SecretKind::MistralKey))
}

/// Removes the key lines from a `.env` file, keeping every other line and the file's permissions.
///
/// # Errors
/// I/O errors.
pub fn remove_dotenv_key(path: &Path) -> std::io::Result<bool> {
    let text = zeroize::Zeroizing::new(std::fs::read_to_string(path)?);
    let mut removed = false;
    let mut kept = zeroize::Zeroizing::new(String::with_capacity(text.len()));
    for line in text.lines() {
        if dotenv_line_key(line).is_some_and(|(name, _)| name == VIBE_KEY_NAME) {
            removed = true;
        } else {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if removed {
        let tmp = path.with_extension("vibe-work-tmp");
        std::fs::write(&tmp, kept.as_bytes())?;
        std::fs::set_permissions(&tmp, std::fs::metadata(path)?.permissions())?;
        std::fs::rename(&tmp, path)?;
    }
    Ok(removed)
}

impl ExistingKeys {
    /// The service holding a key, current first.
    fn keyring_service(&self) -> Option<&'static str> {
        [VIBE_KEYRING_SERVICE, VIBE_LEGACY_KEYRING_SERVICE]
            .into_iter()
            .find(|service| matches!(self.keyring.get(service), Ok(Some(key)) if !key.is_empty()))
    }

    /// Where a key exists (no values).
    #[must_use]
    pub fn detect(&self) -> Vec<ExistingKey> {
        let mut found = Vec::new();
        if read_dotenv_key(&self.dotenv).is_some() {
            found.push(ExistingKey {
                source: ExistingSource::DotEnv,
                location: self.dotenv.display().to_string(),
                mac_keychain: false,
            });
        }
        if let Some(service) = self.keyring_service() {
            found.push(ExistingKey {
                source: ExistingSource::Keyring,
                location: service.to_owned(),
                mac_keychain: cfg!(target_os = "macos"),
            });
        }
        found
    }

    /// The value from `source`.
    #[must_use]
    pub fn read(&self, source: ExistingSource) -> Option<SecretString> {
        match source {
            ExistingSource::DotEnv => read_dotenv_key(&self.dotenv),
            ExistingSource::Keyring => self
                .keyring_service()
                .and_then(|service| self.keyring.get(service).ok().flatten()),
        }
    }

    /// Deletes the original at `source` (both keyring services for the keyring).
    ///
    /// # Errors
    /// A text naming what failed.
    pub fn remove(&self, source: ExistingSource) -> Result<(), String> {
        match source {
            ExistingSource::DotEnv => remove_dotenv_key(&self.dotenv)
                .map(|_| ())
                .map_err(|e| format!("could not edit {}: {e}", self.dotenv.display())),
            ExistingSource::Keyring => {
                for service in [VIBE_KEYRING_SERVICE, VIBE_LEGACY_KEYRING_SERVICE] {
                    self.keyring.delete(service)?;
                }
                Ok(())
            }
        }
    }
}
