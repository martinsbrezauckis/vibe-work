//! Synthetic Vault profiles, fast KDF settings, and an isolated client for integration tests.
//!
//! Every helper forbids the user's real Vault profile first (`forbid_canonical_profile`), so no test
//! can reach it (the real profile holds other tools' test secrets).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vw_vault::location::{VaultLocation, forbid_canonical_profile};
use vw_vault::remember::{DeviceSecretStore, MemoryDeviceStore};
use vw_vault::{KdfParams, SecretString, Vault, VaultConfig};

use crate::common::processes::scratch;

/// Argon2 at Vault's accepted minimum (19 MiB, 1 pass): real format, fast tests.
pub fn fast_kdf() -> KdfParams {
    KdfParams {
        m_cost_kib: 19 * 1024,
        t_cost: 1,
        p_cost: 1,
    }
}

/// A fresh profile folder under the test scratch area (after forbidding the real one).
pub fn profile_dir(name: &str) -> PathBuf {
    forbid_canonical_profile();
    let dir = scratch(&format!("vault-{name}")).join("shellx-vault");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Vibe Work's client on `dir` with `store` as the remembered-device store.
pub fn client(dir: &Path, store: Arc<dyn DeviceSecretStore>) -> Vault {
    forbid_canonical_profile();
    Vault::new(VaultConfig {
        location: VaultLocation::explicit(dir),
        device_store: store,
        kdf: fast_kdf(),
        device_name: "vibe-work-test".to_owned(),
    })
    .unwrap()
}

/// A new in-memory device store.
pub fn memory_store() -> Arc<dyn DeviceSecretStore> {
    Arc::new(MemoryDeviceStore::default())
}

/// A test passphrase (not a real secret).
pub fn passphrase() -> SecretString {
    SecretString::new("correct horse battery staple (test only)")
}

/// The ShellX-layout `profile.json` of `dir` as JSON.
pub fn profile_json(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir.join("profile.json")).unwrap()).unwrap()
}

/// A key-service config on a scratch profile (in `dir`), memory stores, the proxy's upstreams and
/// the test call pointed at `upstream` (a fake Mistral), Vibe's own key looked up in
/// `dir/vibe-home/.env` and `keyring`.
pub fn key_service_config(
    dir: &Path,
    upstream: &str,
    keyring: Arc<vw_keys::existing::MemoryVibeKeyring>,
) -> vw_keys::KeyServiceConfig {
    forbid_canonical_profile();
    vw_keys::KeyServiceConfig {
        managed_vault: None,
        vault: VaultConfig {
            location: VaultLocation::explicit(dir.join("shellx-vault")),
            device_store: memory_store(),
            kdf: fast_kdf(),
            device_name: "vibe-work-test".to_owned(),
        },
        proxy: vw_keyproxy::ProxyConfig {
            upstreams: vw_keyproxy::Upstreams {
                api: upstream.to_owned(),
                console: format!("{upstream}/console-host"),
                chat: format!("{upstream}/chat-host"),
            },
            key_wait: std::time::Duration::from_secs(10),
            ..vw_keyproxy::ProxyConfig::default()
        },
        test_api_base: upstream.to_owned(),
        settings_path: dir.join("keys.json"),
        existing: vw_keys::ExistingKeys {
            dotenv: dir.join("vibe-home/.env"),
            keyring,
        },
    }
}
