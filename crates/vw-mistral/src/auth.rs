//! The `Authorization` header of every call the core makes with the Mistral key: the
//! `Bearer <key>` text is built in a [`Zeroizing`] string (wiped when dropped) and the header value is
//! marked sensitive (reqwest and hyper then keep it out of `Debug` output).
//!
//! Depends on: reqwest's `HeaderValue`, zeroize. Called by: `ocr`, `test_call`, `tts`, `stt_batch`,
//! `realtime`.
//! Invariant: the key is never formatted into anything but this header.

use reqwest::header::HeaderValue;
use zeroize::Zeroizing;

/// `Bearer <key>` as a sensitive header value, or `None` when the key holds characters a header cannot
/// carry (the caller answers with its own "malformed key" error, never echoing the key).
#[must_use]
pub fn bearer(key: &str) -> Option<HeaderValue> {
    let text = Zeroizing::new(format!("Bearer {key}"));
    let mut value = HeaderValue::from_str(text.as_str()).ok()?;
    value.set_sensitive(true);
    Some(value)
}
