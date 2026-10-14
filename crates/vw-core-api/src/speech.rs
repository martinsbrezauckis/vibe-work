//! The `speech` command: dictation streams
//! (start, append, stop, cancel), text-to-speech and the voices list. Every Mistral call is made by the
//! key service with the key from the vault, never by the webview.
//!
//! Depends on: `dictation` (streams), vw-keys (`KeyService` speech calls), vw-mistral (`audio`, limits),
//! `speech_view`, `core` (`Shared`). Called by: `dispatch`.
//! Invariants:
//! - Every origin runs `start` and `speak` directly (the Debug API acts as the user; its
//!   dialog is gone). The starting origin is logged; either origin may go on with any stream.
//! - Without a key, `start` fails at once (the unlock/setup prompt is raised), so the microphone is never
//!   opened for nothing.
//! - Over-long text is refused with the limit in words, never cut silently.
//! - Logs carry ids, sizes, modes and outcomes only: never audio, text or the key.

use std::sync::Arc;

use base64::Engine as _;
use tracing::{info, warn};
use vw_keys::SpeechFailure;
use vw_mistral::audio::{MAX_CHUNK_BYTES, Resampler, STT_SAMPLE_RATE, pcm16_from_le, wav_pcm16};
use vw_mistral::tts::{DEFAULT_VOICE, MAX_TTS_CHARS};

use crate::command::{CommandResult, Origin};
use crate::core::{Core, Shared};
use crate::dictation::StreamError;
use crate::error::{CoreError, ErrorCode};
use crate::speech_view::{DictationMode, SpeechRequest, SpeechResult, VoiceView};

/// Dictation languages a user can fix (the 13 of Voxtral Mini Transcribe 2, `speech_to_text.md`).
pub const LANGUAGES: [&str; 13] = [
    "en", "zh", "hi", "es", "ar", "fr", "pt", "ru", "de", "ja", "ko", "it", "nl",
];

fn result(result: SpeechResult) -> CommandResult {
    CommandResult::Speech { result }
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

fn keys(shared: &Shared) -> Result<Arc<vw_keys::KeyService>, CoreError> {
    shared.config.keys.clone().ok_or_else(|| {
        let mut error = CoreError::new(
            ErrorCode::KeyService,
            "this run has no key service, so voice cannot reach Mistral",
        );
        error.server_code = Some("notConfigured".to_owned());
        error
    })
}

/// A failed Mistral call as a core error (`keyService` with `keyNeeded`, or `serverError` with Mistral's
/// status sentence).
fn failure(error: &SpeechFailure) -> CoreError {
    match error {
        SpeechFailure::KeyNeeded(_) => {
            let mut e = CoreError::new(ErrorCode::KeyService, error.to_string());
            e.server_code = Some("keyNeeded".to_owned());
            e
        }
        SpeechFailure::Speech(_) => {
            let mut e = CoreError::new(ErrorCode::ServerError, error.to_string());
            e.server_code = Some("speech".to_owned());
            e
        }
    }
}

fn stream_error(error: StreamError) -> CoreError {
    match error {
        StreamError::Unknown => CoreError::new(
            ErrorCode::NotFound,
            "no such dictation (it was stopped, cancelled or ended)",
        ),
    }
}

fn start(
    shared: &Shared,
    origin: Origin,
    sample_rate: u32,
    language: Option<String>,
) -> Result<CommandResult, CoreError> {
    let resampler = Resampler::new(sample_rate).map_err(|e| invalid(e.to_string()))?;
    if let Some(lang) = &language
        && !LANGUAGES.contains(&lang.as_str())
    {
        return Err(invalid(format!(
            "dictation language {lang:?} is not one Voxtral transcribes"
        )));
    }
    let keys = keys(shared)?;
    keys.speech_ready().map_err(|e| failure(&e))?;
    let dictations = &shared.dictations;
    let (stream_id, mode) = dictations.start(origin, resampler, language, Some(keys));
    Ok(result(SpeechResult::Started {
        stream_id,
        mode,
        max_seconds: dictations.config().max_seconds,
    }))
}

fn append(shared: &Shared, id: &str, pcm: &str) -> Result<CommandResult, CoreError> {
    // Base64 of 256 KB is under 350,000 characters: refuse before decoding anything larger.
    if pcm.len() > MAX_CHUNK_BYTES / 3 * 4 + 8 {
        return Err(invalid("an audio chunk was larger than 256 KB"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(pcm)
        .map_err(|_| invalid("the audio chunk is not base64"))?;
    let samples = pcm16_from_le(&bytes).map_err(|e| invalid(e.to_string()))?;
    let (seconds, at_limit) = shared
        .dictations
        .append(id, &samples)
        .map_err(stream_error)?;
    Ok(result(SpeechResult::Appended { seconds, at_limit }))
}

async fn stop(shared: &Shared, id: &str) -> Result<CommandResult, CoreError> {
    let stopped = shared.dictations.take_for_stop(id).map_err(stream_error)?;
    let seconds = vw_mistral::audio::seconds_at_16k(stopped.samples.len());
    let wait = shared.dictations.config().stop_wait;
    let mut note = stopped.live_lost.clone();
    if let Some(live) = stopped.live {
        let outcome = tokio::time::timeout(wait, live).await;
        if let Some(task) = stopped.task {
            task.abort();
        }
        match outcome {
            Ok(Ok(Ok((text, model)))) => {
                info!(
                    stream = id,
                    seconds,
                    chars = text.len(),
                    "dictation finished (live)"
                );
                return Ok(result(SpeechResult::Transcript {
                    text,
                    via: DictationMode::Realtime,
                    model,
                    seconds,
                    note: None,
                }));
            }
            Ok(Ok(Err(message))) => note = Some(message),
            Ok(Err(_)) => note = Some("the live transcription stopped".to_owned()),
            Err(_) => note = Some("the live text did not finish in time".to_owned()),
        }
    }
    if stopped.samples.is_empty() {
        return Ok(result(SpeechResult::Transcript {
            text: String::new(),
            via: stopped.mode,
            model: String::new(),
            seconds,
            note,
        }));
    }
    let keys = keys(shared)?;
    let wav = wav_pcm16(&stopped.samples, STT_SAMPLE_RATE);
    let transcript = keys
        .transcribe(&wav, stopped.language.as_deref())
        .await
        .map_err(|e| {
            warn!(stream = id, error = %e, "dictation batch transcription failed");
            failure(&e)
        })?;
    info!(
        stream = id,
        seconds,
        chars = transcript.text.len(),
        "dictation finished (batch)"
    );
    Ok(result(SpeechResult::Transcript {
        text: transcript.text,
        via: DictationMode::Batch,
        model: transcript.model,
        seconds,
        note: if stopped.mode == DictationMode::Realtime {
            note.map(|n| {
                format!(
                    "Live text was not available ({n}); the text was transcribed when you stopped."
                )
            })
        } else {
            None
        },
    }))
}

async fn speak(
    shared: &Shared,
    text: &str,
    voice: Option<String>,
) -> Result<CommandResult, CoreError> {
    let text = text.trim();
    let chars = text.chars().count();
    if chars == 0 {
        return Err(invalid("there is no text to speak"));
    }
    if chars > MAX_TTS_CHARS {
        return Err(invalid(format!(
            "the text is {chars} characters; speech takes at most {MAX_TTS_CHARS} at a time"
        )));
    }
    let voice = voice
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_VOICE.to_owned());
    if voice.len() > 128
        || !voice
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(invalid("that is not a voice id"));
    }
    let keys = keys(shared)?;
    let speech = keys.speak(text, &voice).await.map_err(|e| failure(&e))?;
    info!(chars, bytes = speech.audio.len(), "speech made");
    Ok(result(SpeechResult::Speech {
        mime: speech.mime.to_owned(),
        audio: base64::engine::general_purpose::STANDARD.encode(&speech.audio),
        chars: u32::try_from(chars).unwrap_or(u32::MAX),
    }))
}

impl Core {
    /// The `speech` command (see [`execute`]).
    pub(crate) async fn speech(
        &self,
        origin: Origin,
        request: SpeechRequest,
    ) -> Result<CommandResult, CoreError> {
        execute(&self.shared, origin, request).await
    }
}

/// Executes one speech request.
///
/// # Errors
/// `invalidParams` (limits, formats), `notFound` for a stream that is gone,
/// `confirmationDeclined`, `keyService` (no key service; `keyNeeded`), `serverError` (Mistral's answer).
pub(crate) async fn execute(
    shared: &Shared,
    origin: Origin,
    request: SpeechRequest,
) -> Result<CommandResult, CoreError> {
    match request {
        SpeechRequest::Start {
            sample_rate,
            language,
        } => start(shared, origin, sample_rate, language),
        SpeechRequest::Append { stream_id, pcm } => append(shared, &stream_id, &pcm),
        SpeechRequest::Stop { stream_id } => stop(shared, &stream_id).await,
        SpeechRequest::Cancel { stream_id } => Ok(result(SpeechResult::Cancelled {
            cancelled: shared.dictations.cancel(&stream_id),
        })),
        SpeechRequest::Speak { text, voice } => speak(shared, &text, voice).await,
        SpeechRequest::Voices {} => {
            let voices = keys(shared)?.voices().await.map_err(|e| failure(&e))?;
            Ok(result(SpeechResult::Voices {
                voices: voices.into_iter().map(VoiceView::from).collect(),
            }))
        }
    }
}
