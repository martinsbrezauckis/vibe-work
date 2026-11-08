//! Vault-backed bearer authentication and nonsecret per-run listener discovery.
//! The server caches the token after unlock so existing tools keep working after a vault lock.
//! Development fixtures may explicitly export a plaintext token; release builds contain no export path.
//! Depends on vw-keys/vw-vault and private file helpers; server and auth call this module.

use crate::{auth::constant_time_eq, server::DebugApiError};
use serde_json::json;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
use vw_platform::{ensure_private_dir, is_user_only, write_private_new};
use vw_vault::SecretString;

/// Legacy/development fixture file name. Release startup removes this file without using its value.
pub const TOKEN_FILE: &str = "debug-api.token";

/// Removes plaintext credentials left by an older version, even when the API is disabled or the vault locked.
/// # Errors
/// Returns an I/O error if an existing plaintext credential cannot be removed.
pub fn remove_legacy_token(dir: &Path) -> Result<(), DebugApiError> {
    for name in [TOKEN_FILE.to_owned(), format!("{TOKEN_FILE}.new")] {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(DebugApiError::Descriptor {
                    path: path.display().to_string(),
                    source,
                });
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct TokenStore {
    keys: Option<Arc<vw_keys::KeyService>>,
    test_path: Option<Arc<PathBuf>>,
    current: Arc<RwLock<SecretString>>,
    rotation: Arc<tokio::sync::Mutex<()>>,
}
impl fmt::Debug for TokenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenStore(<redacted>)")
    }
}
impl TokenStore {
    pub(crate) async fn load_or_create(
        dir: &Path,
        keys: Option<Arc<vw_keys::KeyService>>,
        test_file: bool,
    ) -> Result<Self, DebugApiError> {
        // Fixture drivers must be able to create/unlock their synthetic vault through native input.
        // Their explicit opt-in selects a separate throwaway credential, never a real vault fallback.
        #[cfg(debug_assertions)]
        if test_file {
            return Self::fixture(dir);
        }
        let current = match &keys {
            Some(keys) => keys.debug_api_token(false).await?,
            None => {
                return Err(DebugApiError::VaultUnavailable);
            }
        };
        #[cfg(not(debug_assertions))]
        let _ = (dir, test_file);
        current.register_as(vw_secrets::SecretKind::DebugApiToken);
        Ok(Self {
            keys,
            test_path: None,
            current: Arc::new(RwLock::new(current)),
            rotation: Arc::default(),
        })
    }
    #[cfg(debug_assertions)]
    fn fixture(dir: &Path) -> Result<Self, DebugApiError> {
        let current = crate::test_token::load_or_create(dir)?;
        current.register_as(vw_secrets::SecretKind::DebugApiToken);
        Ok(Self {
            keys: None,
            test_path: Some(Arc::new(dir.join(TOKEN_FILE))),
            current: Arc::new(RwLock::new(current)),
            rotation: Arc::default(),
        })
    }
    pub(crate) fn path(&self) -> Option<&Path> {
        self.test_path.as_deref().map(PathBuf::as_path)
    }
    pub(crate) fn matches(&self, given: &[u8]) -> bool {
        let current = self.current.read().unwrap_or_else(PoisonError::into_inner);
        constant_time_eq(given, current.expose().as_bytes())
    }
    pub(crate) async fn rotate(&self) -> Result<(), DebugApiError> {
        let _rotation = self.rotation.lock().await;
        let fresh = if let Some(keys) = &self.keys {
            keys.debug_api_token(true).await?
        } else {
            #[cfg(debug_assertions)]
            if let Some(path) = &self.test_path {
                let dir = path.parent().ok_or(DebugApiError::VaultUnavailable)?;
                crate::test_token::rotate_token_file(dir)?;
                let fresh = crate::test_token::load_or_create(dir)?;
                *self.current.write().unwrap_or_else(PoisonError::into_inner) = fresh;
                return Ok(());
            }
            return Err(DebugApiError::VaultUnavailable);
        };
        fresh.register_as(vw_secrets::SecretKind::DebugApiToken);
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = fresh;
        Ok(())
    }
}

/// The descriptor's file name for this process.
pub(crate) fn descriptor_path(dir: &Path) -> PathBuf {
    dir.join(format!("debug-api-{}.json", std::process::id()))
}

/// Writes `{url, port, pid, startedAt}` (plus `tokenFile` for development fixtures) to a new user-only file in `dir` and verifies
/// it. A stale file of an earlier process with the same id (pid reuse) is replaced. The token
/// itself is not in it.
pub(crate) fn write_descriptor(
    dir: &Path,
    port: u16,
    token_file: Option<&Path>,
) -> Result<PathBuf, DebugApiError> {
    ensure_private_dir(dir)?;
    let path = descriptor_path(dir);
    if std::fs::symlink_metadata(&path).is_ok() {
        std::fs::remove_file(&path).map_err(|source| DebugApiError::Descriptor {
            path: path.display().to_string(),
            source,
        })?;
    }
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let mut body = json!({
        "url": format!("http://127.0.0.1:{port}"),
        "port": port,
        "pid": std::process::id(),
        "startedAt": u64::try_from(started).unwrap_or(u64::MAX),
    });
    if let Some(token_file) = token_file {
        body["tokenFile"] = json!(token_file.display().to_string());
    }
    write_private_new(&path, format!("{body:#}\n").as_bytes())?;
    if !is_user_only(&path)? {
        let _ = std::fs::remove_file(&path);
        return Err(DebugApiError::NotPrivate(path.display().to_string()));
    }
    Ok(path)
}
