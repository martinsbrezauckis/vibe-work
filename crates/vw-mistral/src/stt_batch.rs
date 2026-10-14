//! Batch transcription (`POST /v1/audio/transcriptions`, Voxtral Mini Transcribe 2),. Used when
//! the user fixed a dictation language (realtime cannot force one: live probe)
//! and as the fallback when the realtime connection fails.
//! Source: docs.mistral.ai `studio/audio/speech_to_text/offline_transcription.md` (multipart form `file`,
//! `model`, optional `language`; answer `{model, text, language,...}`).
//!
//! The multipart body is built here (one file part, two text fields) instead of turning on reqwest's
//! `multipart` feature: the format is small and fixed, and the test checks every byte of it.
//!
//! Depends on: reqwest, `serde_json`, `auth`, `tts` (shared error type and helpers). Called by: vw-keys.
//! Invariants: the key only in the `Authorization` header; error texts never contain a body.

use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

use crate::tts::{SpeechError, network, read_capped, status_error};

/// The batch transcription model alias (the docs' `voxtral-mini-latest`, which runs Voxtral Mini
/// Transcribe 2).
pub const BATCH_STT_MODEL: &str = "voxtral-mini-latest";

/// How long one batch transcription may take (minutes of audio take seconds).
pub const BATCH_TIMEOUT: Duration = Duration::from_secs(120);

/// What a transcription returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    /// The text, trimmed.
    pub text: String,
    /// The model Mistral answered with.
    pub model: String,
    /// The language Mistral reported, when it did.
    pub language: Option<String>,
}

/// The multipart boundary: fixed and long enough never to appear inside a WAV of our own making by
/// accident in practice; [`multipart_body`] still checks.
const BOUNDARY: &str = "vw-dictation-7c1f9a3e5b2d48a6b0e4d1c3f5a7b9e2";

/// Builds the multipart body and its content type. `None` if the boundary occurs in the audio (the caller
/// then reports a network-style error; with 16-bit audio this is practically impossible).
#[must_use]
pub fn multipart_body(wav: &[u8], language: Option<&str>) -> Option<(Vec<u8>, String)> {
    if wav
        .windows(BOUNDARY.len())
        .any(|w| w == BOUNDARY.as_bytes())
    {
        return None;
    }
    let mut body = Vec::with_capacity(wav.len() + 512);
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    field("model", BATCH_STT_MODEL);
    if let Some(lang) = language {
        field("language", lang);
    }
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"dictation.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    Some((body, format!("multipart/form-data; boundary={BOUNDARY}")))
}

/// Transcribes one WAV file at `api_base` with `key`; `language` is an ISO code (`en`, `fr`) or `None`
/// for detection.
///
/// # Errors
/// [`SpeechError`] (see its variants).
pub async fn transcribe(
    client: &reqwest::Client,
    api_base: &str,
    key: &str,
    wav: &[u8],
    language: Option<&str>,
    timeout: Duration,
) -> Result<Transcript, SpeechError> {
    let auth = crate::auth::bearer(key).ok_or(SpeechError::MalformedKey)?;
    let (body, content_type) = multipart_body(wav, language)
        .ok_or_else(|| SpeechError::Network("the audio could not be packed".to_owned()))?;
    let response = client
        .post(format!(
            "{}/v1/audio/transcriptions",
            api_base.trim_end_matches('/')
        ))
        .header(AUTHORIZATION, auth)
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| network(&e))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(status_error(status));
    }
    let bytes = read_capped(response, 8 * 1024 * 1024).await?;
    let unreadable = || SpeechError::Network("the answer could not be read".to_owned());
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| unreadable())?;
    let text = body
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(unreadable)?;
    Ok(Transcript {
        text: text.trim().to_owned(),
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(BATCH_STT_MODEL)
            .to_owned(),
        language: body
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}
