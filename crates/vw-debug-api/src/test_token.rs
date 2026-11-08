//! Plaintext token export for explicitly selected test fixtures in development builds only.
//! Depends on private file helpers; the token store calls this. This module is absent from release builds.
use crate::{server::DebugApiError, token::TOKEN_FILE};
use std::path::{Path, PathBuf};
use vw_platform::{ensure_private_dir, is_user_only, write_private_new};
use vw_vault::SecretString;

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

pub(crate) fn load_or_create(dir: &Path) -> Result<SecretString, DebugApiError> {
    ensure_private_dir(dir)?;
    let path = dir.join(TOKEN_FILE);
    if let Some(token) = reusable(&path) {
        return Ok(SecretString::new(token));
    }
    let token = generate()?;
    write_atomic(dir, &path, &token)?;
    Ok(SecretString::new(token))
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
