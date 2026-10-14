//! The stable, rotatable bearer token and the descriptor file that hands the port and the token
//! file's location to local clients.
//!
//! Model (decided, adapted from ShellX `debug_api_auth.rs` @ e38c2b5c: one token file
//! per user, created once, reused, regenerated on request): an agent set up once keeps working
//! across app restarts. The token is the gate, not secrecy of the port: a script in the webview can
//! switch the API on but cannot read the token file.
//!
//! Depends on: `getrandom` (OS CSPRNG), vw-platform (`ensure_private_dir`, `write_private_new`,
//! `is_user_only`), `serde_json`. Called by: `server` (start, rotate, shutdown), `auth` (checks).
//! Invariants:
//! - 32 random bytes from the OS source only, as 64 lowercase hex; no environment override.
//! - The token lives in `<dir>/debug-api.token`, a user-only file in a user-only directory. An
//!   existing file is reused only when it is a plain file, user-only and well formed; anything else
//!   is replaced by a fresh token (never used as is). Writes are atomic (temp file + rename) and
//!   read back for permissions; a failure stops the start (fail closed: no memory-only token, no
//!   world-readable file).
//! - The token is held in memory per process and never re-read per request; [`TokenStore::rotate`]
//!   swaps it at once, so the old token gets 401 on the very next request.
//! - The token is never in the descriptor, in logs (`Debug` prints a placeholder) or in any route
//!   output; the descriptor carries only the token file's path.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use vw_platform::{ensure_private_dir, is_user_only, write_private_new};

use crate::auth::constant_time_eq;
use crate::server::DebugApiError;

/// File name of the token inside the descriptor directory.
pub const TOKEN_FILE: &str = "debug-api.token";

/// The current token and where it is stored.
#[derive(Clone)]
pub(crate) struct TokenStore {
    path: Arc<PathBuf>,
    current: Arc<RwLock<Arc<str>>>,
}

impl fmt::Debug for TokenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenStore(<redacted>)")
    }
}

/// 32 random bytes as 64 lowercase hex characters.
fn generate() -> Result<String, DebugApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| DebugApiError::Entropy(e.to_string()))?;
    let mut text = String::with_capacity(64);
    for byte in bytes {
        for nibble in [byte >> 4, byte & 0x0f] {
            text.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
        }
    }
    Ok(text)
}

fn well_formed(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Writes `token` to `path` atomically as a user-only file and verifies it.
fn write_atomic(dir: &Path, path: &Path, token: &str) -> Result<(), DebugApiError> {
    ensure_private_dir(dir)?;
    let temp = dir.join(format!("{TOKEN_FILE}.new"));
    if std::fs::symlink_metadata(&temp).is_ok() {
        std::fs::remove_file(&temp).map_err(|source| DebugApiError::Descriptor {
            path: temp.display().to_string(),
            source,
        })?;
    }
    write_private_new(&temp, format!("{token}\n").as_bytes())?;
    if !is_user_only(&temp)? {
        let _ = std::fs::remove_file(&temp);
        return Err(DebugApiError::NotPrivate(temp.display().to_string()));
    }
    std::fs::rename(&temp, path).map_err(|source| {
        let _ = std::fs::remove_file(&temp);
        DebugApiError::Descriptor {
            path: path.display().to_string(),
            source,
        }
    })?;
    if !is_user_only(path)? {
        let _ = std::fs::remove_file(path);
        return Err(DebugApiError::NotPrivate(path.display().to_string()));
    }
    Ok(())
}

/// The existing token when the file is usable as is.
fn reusable(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || !is_user_only(path).ok()? {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let token = text.trim();
    well_formed(token).then(|| token.to_owned())
}

impl TokenStore {
    /// Loads the user's token from `dir`, creating it on the first start.
    pub(crate) fn load_or_create(dir: &Path) -> Result<Self, DebugApiError> {
        ensure_private_dir(dir)?;
        let path = dir.join(TOKEN_FILE);
        let token = if let Some(token) = reusable(&path) {
            token
        } else {
            let fresh = generate()?;
            write_atomic(dir, &path, &fresh)?;
            fresh
        };
        // the token is a stored secret; the response guard refuses any answer that holds it.
        vw_secrets::register(vw_secrets::SecretKind::DebugApiToken, &token);
        Ok(Self {
            path: Arc::new(path),
            current: Arc::new(RwLock::new(token.into())),
        })
    }

    /// The token file.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// True when `given` is the current token (constant time; the length is public).
    pub(crate) fn matches(&self, given: &[u8]) -> bool {
        let current = self
            .current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        constant_time_eq(given, current.as_bytes())
    }

    /// Replaces the token: a new one is written atomically, then swapped in memory. The old token
    /// stops working with the swap.
    pub(crate) fn rotate(&self) -> Result<(), DebugApiError> {
        let fresh = generate()?;
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        write_atomic(dir, &self.path, &fresh)?;
        vw_secrets::register(vw_secrets::SecretKind::DebugApiToken, &fresh);
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = fresh.into();
        Ok(())
    }
}

/// Writes a fresh token file in `dir` without a running server (the user regenerates while the
/// API is off); the next start reuses it. Returns the token file's path.
///
/// # Errors
/// [`DebugApiError`] when the directory or file cannot be made user-only.
pub fn rotate_token_file(dir: &Path) -> Result<PathBuf, DebugApiError> {
    let path = dir.join(TOKEN_FILE);
    let fresh = generate()?;
    write_atomic(dir, &path, &fresh)?;
    vw_secrets::register(vw_secrets::SecretKind::DebugApiToken, &fresh);
    Ok(path)
}

/// The descriptor's file name for this process.
pub(crate) fn descriptor_path(dir: &Path) -> PathBuf {
    dir.join(format!("debug-api-{}.json", std::process::id()))
}

/// Writes `{url, port, tokenFile, pid, startedAt}` to a new user-only file in `dir` and verifies
/// it. A stale file of an earlier process with the same id (pid reuse) is replaced. The token
/// itself is not in it.
pub(crate) fn write_descriptor(
    dir: &Path,
    port: u16,
    token_file: &Path,
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
    let body = json!({
        "url": format!("http://127.0.0.1:{port}"),
        "port": port,
        "tokenFile": token_file.display().to_string(),
        "pid": std::process::id(),
        "startedAt": u64::try_from(started).unwrap_or(u64::MAX),
    });
    write_private_new(&path, format!("{body:#}\n").as_bytes())?;
    if !is_user_only(&path)? {
        let _ = std::fs::remove_file(&path);
        return Err(DebugApiError::NotPrivate(path.display().to_string()));
    }
    Ok(path)
}
