//! `SecretString`: a string secret (API key, passphrase, device secret) that is zeroed when dropped
//! and never shown by `Debug` or `Display`.
//!
//! Depends on: zeroize, vw-secrets (stored values are registered there). Called by: every module that
//! touches a secret, and vw-keys / vw-core-api (command inputs).
//! Invariant: wrapping a value does not register it. A value that is stored or read from storage is registered by
//! the code that stores or reads it ([`SecretString::register_as`]), so an unconfirmed input (a wrong passphrase, a
//! key that failed its test) never enters the Debug API's redaction list.

use std::fmt;

use zeroize::Zeroizing;

/// A secret string. `Clone` copies into a new zeroizing buffer; `Debug` prints `<redacted>`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    /// Wraps a value (surrounding whitespace removed: keys and passphrases pasted with a newline
    /// must still match).
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let mut raw: String = value.into();
        let trimmed = raw.trim().to_owned();
        zeroize::Zeroize::zeroize(&mut raw);
        Self(Zeroizing::new(trimmed))
    }

    /// Wraps a value exactly as given (a website password may begin or end with a space, and a trimmed
    /// one would sign in with the wrong value).
    #[must_use]
    pub fn exact(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// Registers the value as a stored secret of `kind` (vw-secrets: the Debug API's answers never carry it).
    /// Called where a value is stored or read from storage, never for an unconfirmed input.
    pub fn register_as(&self, kind: vw_secrets::SecretKind) {
        vw_secrets::register(kind, &self.0);
    }

    /// The value. Callers keep the borrow short and never log or format it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when empty after trimming.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Length in bytes (for tests and checks that must not print the value).
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}
