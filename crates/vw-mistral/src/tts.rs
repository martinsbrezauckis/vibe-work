//! Voxtral text-to-speech (`POST /v1/audio/speech`) and the voices list (`GET /v1/audio/voices`), block
//! Source: docs.mistral.ai `studio/audio/text_to_speech/{speech,voices}.md` and
//! the `mistralai` 2.6.0 SDK models (`SpeechRequest`, `VoiceResponse`).
//!
//! Choice: model alias `voxtral-mini-tts-latest` (Vibe 2.26.0's default, `vibe_schema.py:187`), WAV output,
//! one non-streaming request (narration texts are a sentence or two). The voice is a preset slug
//! (`gb_jane_neutral`, Vibe's default) or a custom voice id.
//!
//! Depends on: reqwest, base64, `serde_json`, `auth`. Called by: vw-keys (`KeyService::speak|voices`).
//! Invariants: the key only in the `Authorization` header (`auth::bearer`); error texts carry a status and
//! a fixed sentence, never a response body (a 4xx body may echo the text); the audio answer is capped.

use std::time::Duration;

use base64::Engine as _;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;
use serde_json::{Value, json};

/// The TTS model alias the app uses.
pub const TTS_MODEL: &str = "voxtral-mini-tts-latest";

/// The voice used when the user picked none (Vibe's default narrator voice).
pub const DEFAULT_VOICE: &str = "gb_jane_neutral";

/// Longest text one request may carry, in characters. Mistral advises "under 300 words"; longer text is
/// refused by the caller with a clear sentence, never cut silently.
pub const MAX_TTS_CHARS: usize = 2_000;

/// Largest audio answer accepted (base64 text), 24 MB: minutes of WAV, far above any narration.
const MAX_ANSWER_BYTES: usize = 24 * 1024 * 1024;

/// How long one speech request may take.
pub const TTS_TIMEOUT: Duration = Duration::from_secs(60);

/// What TTS returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    /// The audio file (WAV).
    pub audio: Vec<u8>,
    /// Its MIME type (`audio/wav`).
    pub mime: &'static str,
}

/// One voice of the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    /// What to send as `voice_id`: the slug of a preset, else the id.
    pub voice_id: String,
    /// Display name ("Jane - Neutral").
    pub name: String,
    /// Languages (`en_gb`, `fr`,...).
    pub languages: Vec<String>,
    /// Gender label, when given.
    pub gender: Option<String>,
    /// True for Mistral's presets (no owner), false for the user's own voices.
    pub preset: bool,
}

/// Why a speech call failed. Texts are shown to the user as they are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpeechError {
    /// HTTP 401.
    #[error("Mistral did not accept the API key (401). Check it in Settings.")]
    Unauthorized,
    /// HTTP 403: plan, permission or Mistral's content moderation (the TTS docs answer 403 for refused text).
    #[error(
        "Mistral refused this speech request (403): the key's plan or permissions, or its content moderation."
    )]
    Forbidden,
    /// HTTP 402.
    #[error("This key's workspace cannot use speech right now (402). Check its plan and billing.")]
    Payment,
    /// HTTP 400/422: Mistral could not use the request (unknown voice, bad audio,...).
    #[error("Mistral could not use this request ({0}). The voice or the audio may not be valid.")]
    Rejected(u16),
    /// HTTP 413.
    #[error("Mistral says the audio is too large (413).")]
    TooLarge,
    /// HTTP 429.
    #[error("Mistral is rate-limiting this key right now (429). Try again in a minute.")]
    RateLimited,
    /// HTTP 5xx.
    #[error("Mistral's speech service had a problem ({0}). Try again later.")]
    Server(u16),
    /// Any other status (incl. a redirect, which is never followed).
    #[error("Mistral answered with an unexpected status ({0}).")]
    Unexpected(u16),
    /// No answer, or an answer that could not be read.
    #[error("Could not reach Mistral: {0}.")]
    Network(String),
    /// The key cannot be put in a header.
    #[error("The stored key contains characters a key cannot have.")]
    MalformedKey,
}

/// Maps a non-2xx status to its error.
#[must_use]
pub fn status_error(status: u16) -> SpeechError {
    match status {
        401 => SpeechError::Unauthorized,
        402 => SpeechError::Payment,
        403 => SpeechError::Forbidden,
        413 => SpeechError::TooLarge,
        400 | 422 => SpeechError::Rejected(status),
        429 => SpeechError::RateLimited,
        500..=599 => SpeechError::Server(status),
        other => SpeechError::Unexpected(other),
    }
}

/// A short reason for a transport failure (never the URL or anything from the request).
pub(crate) fn network(error: &reqwest::Error) -> SpeechError {
    SpeechError::Network(
        if error.is_timeout() {
            "the request timed out"
        } else if error.is_connect() {
            "the connection failed"
        } else {
            "the request failed"
        }
        .to_owned(),
    )
}

/// The request body (no key in it).
#[must_use]
pub fn speech_body(text: &str, voice: &str) -> Value {
    json!({"model": TTS_MODEL, "input": text, "voice_id": voice, "response_format": "wav"})
}

/// Reads a body of at most `cap` bytes.
pub(crate) async fn read_capped(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<Vec<u8>, SpeechError> {
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| network(&e))? {
        if out.len() + chunk.len() > cap {
            return Err(SpeechError::Network("the answer was too large".to_owned()));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Speaks `text` with `voice` at `api_base` (normally `https://api.mistral.ai`) using `key`.
///
/// # Errors
/// [`SpeechError`] (see its variants).
pub async fn speak(
    client: &reqwest::Client,
    api_base: &str,
    key: &str,
    text: &str,
    voice: &str,
    timeout: Duration,
) -> Result<Speech, SpeechError> {
    let auth = crate::auth::bearer(key).ok_or(SpeechError::MalformedKey)?;
    let response = client
        .post(format!(
            "{}/v1/audio/speech",
            api_base.trim_end_matches('/')
        ))
        .header(AUTHORIZATION, auth)
        .header(CONTENT_TYPE, "application/json")
        .body(speech_body(text, voice).to_string())
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| network(&e))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(status_error(status));
    }
    let bytes = read_capped(response, MAX_ANSWER_BYTES).await?;
    let unreadable = || SpeechError::Network("the answer could not be read".to_owned());
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| unreadable())?;
    let data = body
        .get("audio_data")
        .and_then(Value::as_str)
        .ok_or_else(unreadable)?;
    let audio = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| unreadable())?;
    if audio.is_empty() {
        return Err(unreadable());
    }
    Ok(Speech {
        audio,
        mime: "audio/wav",
    })
}

fn voice_of(item: &Value) -> Option<Voice> {
    let id = item.get("id").and_then(Value::as_str)?;
    let slug = item
        .get("slug")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    Some(Voice {
        voice_id: slug.unwrap_or(id).to_owned(),
        name: item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_owned(),
        languages: item
            .get("languages")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        gender: item
            .get("gender")
            .and_then(Value::as_str)
            .map(str::to_owned),
        preset: item.get("user_id").is_none_or(Value::is_null),
    })
}

/// Lists every voice the key can use (presets first, then the user's own), following the offset
/// pagination 100 at a time, at most 1,000 voices.
///
/// # Errors
/// [`SpeechError`] (see its variants).
pub async fn voices(
    client: &reqwest::Client,
    api_base: &str,
    key: &str,
) -> Result<Vec<Voice>, SpeechError> {
    let auth = crate::auth::bearer(key).ok_or(SpeechError::MalformedKey)?;
    let mut out: Vec<Voice> = Vec::new();
    let mut offset = 0usize;
    while offset < 1_000 {
        let response = client
            .get(format!(
                "{}/v1/audio/voices?limit=100&offset={offset}&type=all",
                api_base.trim_end_matches('/')
            ))
            .header(AUTHORIZATION, auth.clone())
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| network(&e))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(status_error(status));
        }
        let bytes = read_capped(response, 4 * 1024 * 1024).await?;
        let body: Value = serde_json::from_slice(&bytes)
            .map_err(|_| SpeechError::Network("the answer could not be read".to_owned()))?;
        let items = body
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let total = body
            .get("total")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
        out.extend(items.iter().filter_map(voice_of));
        offset += items.len();
        if items.is_empty() || offset >= total {
            break;
        }
    }
    out.sort_by(|a, b| b.preset.cmp(&a.preset).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}
