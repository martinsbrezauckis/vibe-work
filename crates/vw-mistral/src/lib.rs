//! Direct Mistral API calls made by the app itself, and the HTTPS client every outbound
//! Mistral call of the core uses (the model proxy in vw-keyproxy, the key test call here).
//! OCR, one short chat completion for the core's own text jobs (commit messages) and Voxtral speech (realtime and batch transcription, text-to-speech,
//! voices) are here; rate-limit headers come in a later block.
//!
//! adds `work`: document Libraries and image generation through Mistral's built-in tools.
//!
//! Modules: `client` (HTTPS clients, no redirects), `auth` (the zeroized key header), `test_call`, `ocr`,
//! `audio` (PCM checks, resampling, WAV), `tts` (speech, voices, the shared speech error), `stt_batch`,
//! `realtime` (protocol) over `ws` (WebSocket over our own TLS).
//!
//! Depends on: reqwest (Linux: rustls + ring; Windows/macOS: native TLS; OS trust store),
//! tokio-tungstenite (handshake only), `serde_json`. Called by: vw-keyproxy, vw-keys, vw-core-api (audio).
//! Invariants: the API key is passed in per call as `&str`, put only into a sensitive, zeroized
//! `Authorization` header (`auth`), and never logged or included in an error text; no key-bearing call
//! follows a redirect.

pub mod audio;
mod auth;
pub mod chat;
mod client;
mod ocr;
pub mod realtime;
pub mod stt_batch;
mod test_call;
pub mod tts;
pub mod work;
mod ws;

pub use auth::bearer;
pub use client::{ClientError, https_client, proxy_client, tls_builder};
pub use ocr::{OCR_MODEL, OCR_TIMEOUT, OcrError, OcrInput, OcrOutput, ocr, request_body};
pub use test_call::{DEFAULT_API_BASE, TestCallError, TestCallOk, list_models};
pub use tts::{SpeechError, Voice};

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "direct Mistral API calls";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
