//! The proxy token, which is also the placeholder `MISTRAL_API_KEY` Vibe sees.
//!
//! Shape: 32 ASCII letters and digits, like a real Mistral key, so Vibe's own key checks behave
//! (a dummy shaped like a real key). 32 characters from 62 give about 190 bits.
//! A new token replaces the old one at every server start ([`ProxySecretProvider`]); only the
//! current one is accepted, compared in constant time.
//!
//! Depends on: getrandom, subtle, zeroize, vw-supervisor. Called by: `server`, vw-keys.

use std::sync::{Mutex, PoisonError};

use subtle::ConstantTimeEq;
use vw_supervisor::{ApiKey, SecretError, SecretProvider};
use zeroize::Zeroizing;

/// Length of a placeholder / token.
pub const PLACEHOLDER_LEN: usize = 32;

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// A fresh random token (rejection sampling keeps the 62 symbols uniform).
///
/// # Errors
/// [`SecretError`] when the OS random source fails.
pub fn new_placeholder() -> Result<Zeroizing<String>, SecretError> {
    let mut out = Zeroizing::new(String::with_capacity(PLACEHOLDER_LEN));
    let mut buf = [0u8; 64];
    while out.len() < PLACEHOLDER_LEN {
        getrandom::fill(&mut buf).map_err(|e| SecretError(format!("random source failed: {e}")))?;
        for byte in buf {
            // 248 = 4 * 62: bytes above it would bias the first symbols.
            if byte < 248 && out.len() < PLACEHOLDER_LEN {
                out.push(char::from(ALPHABET[usize::from(byte % 62)]));
            }
        }
    }
    zeroize::Zeroize::zeroize(&mut buf);
    // the token is a stored secret; the Debug API's response guard replaces it wherever it appears.
    vw_secrets::register(vw_secrets::SecretKind::ProxyToken, &out);
    Ok(out)
}

/// The current token. `None` until the first server start.
#[derive(Default)]
pub struct TokenSlot(Mutex<Option<Zeroizing<String>>>);

impl TokenSlot {
    /// Replaces the token with a new one and returns it (for the child's environment only).
    ///
    /// # Errors
    /// [`SecretError`] when the random source fails (the old token stays valid then).
    pub fn rotate(&self) -> Result<Zeroizing<String>, SecretError> {
        let token = new_placeholder()?;
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(token.clone());
        Ok(token)
    }

    /// True when `presented` is the current token (constant-time on equal lengths).
    #[must_use]
    pub fn accepts(&self, presented: &str) -> bool {
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        guard.as_ref().is_some_and(|current| {
            current.len() == presented.len()
                && bool::from(current.as_bytes().ct_eq(presented.as_bytes()))
        })
    }

    /// The current token, if any (the same value goes to the server as the MCP forwarder token).
    #[must_use]
    pub fn current(&self) -> Option<Zeroizing<String>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Clears the token: nothing is accepted until the next rotation.
    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The supervisor's secret provider in product runs: each server start rotates the proxy token and
/// gives the child the new token as `MISTRAL_API_KEY` and as `VW_MCP_TOKEN`. The real key never
/// reaches the child.
pub struct ProxySecretProvider {
    slot: std::sync::Arc<TokenSlot>,
}

impl ProxySecretProvider {
    /// A provider rotating `slot`.
    #[must_use]
    pub fn new(slot: std::sync::Arc<TokenSlot>) -> Self {
        Self { slot }
    }
}

impl SecretProvider for ProxySecretProvider {
    fn mistral_api_key(&self) -> Result<Option<ApiKey>, SecretError> {
        let token = self.slot.rotate()?;
        Ok(Some(ApiKey::new(token.as_str())))
    }

    /// the token just rotated by [`SecretProvider::mistral_api_key`] (the supervisor asks right after
    /// it), which Vibe sends to the MCP forwarder (`mcp.rs`) as `VW_MCP_TOKEN`.
    fn mcp_forward_token(&self) -> Option<ApiKey> {
        self.slot.current().map(|token| ApiKey::new(token.as_str()))
    }
}
