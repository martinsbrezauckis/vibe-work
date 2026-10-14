//! Random identifiers: confirmation tokens and our own `clientUserMessageId` values.
//!
//! Depends on: `getrandom` (OS CSPRNG). Called by: confirm, `session_rules`.
//! Invariant: output comes only from the OS random source; failure is an error, never a fallback.

use crate::error::GateError;

/// `bytes` random bytes as lowercase hex.
///
/// # Errors
/// [`GateError::Entropy`] when the OS random source fails.
pub(crate) fn random_hex(bytes: usize) -> Result<String, GateError> {
    let mut buf = vec![0_u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| GateError::Entropy(e.to_string()))?;
    let mut out = String::with_capacity(bytes * 2);
    for byte in buf {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    Ok(out)
}
