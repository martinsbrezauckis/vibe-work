//! Voxtral realtime transcription over a WebSocket: live partial text while the user speaks.
//! Endpoint `wss://api.mistral.ai/v1/audio/transcriptions/realtime?model=...` (docs.mistral.ai
//! `studio/audio/speech_to_text/realtime_transcription.md`); messages from the `mistralai`
//! 2.6.0 SDK (`extra/realtime/connection.py`, `client/models/realtimetranscription*.py`); handshake,
//! format update and an empty session probed live.
//!
//! Protocol as used here: server `session.created` -> client `session.update {audio_format: pcm_s16le @
//! 16 kHz}` -> client `input_audio.append {audio: base64}` (<= 262,144 decoded bytes each)... -> client
//! `input_audio.flush` + `input_audio.end` -> server `transcription.text.delta {text}` along the way and
//! `transcription.done {text, model}` at the end; `error {error: {message, code}}` at any point. The
//! realtime model cannot force a language (live: error 3051), so no `language` is ever sent.
//!
//! Depends on: `ws`, `auth`, `audio`, base64, `serde_json`, futures-util. Called by: vw-keys
//! (`KeyService::realtime`), driven by vw-core-api `dictation`.
//! Invariants: the key only in the handshake header; an `error` event's message is Mistral's own sentence,
//! cut to 200 characters (it never contains the key: the key is not in any message we send).

use std::time::Duration;

use base64::Engine as _;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use crate::audio::{MAX_CHUNK_BYTES, STT_SAMPLE_RATE};
use crate::tts::SpeechError;
use crate::ws::Socket;

/// The realtime model (the only realtime-capable one in the docs; no `-latest` alias exists for it).
pub const REALTIME_MODEL: &str = "voxtral-mini-transcribe-realtime-2602";

/// How long connecting (TCP, TLS, handshake, `session.created`) may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Longest error text kept from Mistral's `error` event.
const MAX_ERROR_CHARS: usize = 200;

/// Something the realtime connection reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeEvent {
    /// More text (`transcription.text.delta`): append it to what came before.
    Delta(String),
    /// The final transcript (`transcription.done`).
    Done {
        /// The whole text.
        text: String,
        /// The model that transcribed it.
        model: String,
    },
    /// Mistral reported an error (its message, shortened).
    Failed(String),
    /// The connection closed without a final transcript.
    Closed,
}

/// The sending half of a realtime connection.
pub struct RealtimeSender {
    sink: SplitSink<Socket, Message>,
}

/// The receiving half of a realtime connection.
pub struct RealtimeReceiver {
    stream: SplitStream<Socket>,
}

/// The realtime URL for an API base (`https://` -> `wss://`, `http://` -> `ws://`).
#[must_use]
pub fn realtime_url(api_base: &str) -> String {
    let base = api_base.trim_end_matches('/');
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_owned()
    };
    format!("{base}/v1/audio/transcriptions/realtime?model={REALTIME_MODEL}")
}

/// Mistral's error sentence out of an `error` event (`error.message` is a string or `{detail}`).
fn error_message(value: &Value) -> String {
    let message = value.pointer("/error/message");
    let text = message
        .and_then(Value::as_str)
        .or_else(|| {
            message
                .and_then(|m| m.get("detail"))
                .and_then(Value::as_str)
        })
        .unwrap_or("Mistral's live transcription reported an error");
    let mut short: String = text.chars().take(MAX_ERROR_CHARS).collect();
    if text.chars().count() > MAX_ERROR_CHARS {
        short.push('…');
    }
    short
}

/// Reads one server message; `None` for messages this client does not use (language, segments, session
/// updates, unknown types, non-JSON).
#[must_use]
pub fn parse_event(text: &str) -> Option<RealtimeEvent> {
    let value: Value = serde_json::from_str(text).ok()?;
    match value.get("type").and_then(Value::as_str)? {
        "transcription.text.delta" => Some(RealtimeEvent::Delta(
            value.get("text").and_then(Value::as_str)?.to_owned(),
        )),
        "transcription.done" => Some(RealtimeEvent::Done {
            text: value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            model: value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(REALTIME_MODEL)
                .to_owned(),
        }),
        "error" => Some(RealtimeEvent::Failed(error_message(&value))),
        _ => None,
    }
}

/// The `session.update` message that sets the audio format (sent before any audio).
#[must_use]
pub fn format_message() -> String {
    json!({
        "type": "session.update",
        "session": {"audio_format": {"encoding": "pcm_s16le", "sample_rate": STT_SAMPLE_RATE}},
    })
    .to_string()
}

/// Opens a realtime session at `api_base` with `key`: waits for `session.created` and sets the format.
///
/// # Errors
/// [`SpeechError`]: the handshake status (`401`, `403`,...), a network failure, a timeout, or Mistral's
/// `error` before the session started (as [`SpeechError::Network`] with its sentence).
pub async fn connect(
    api_base: &str,
    key: &str,
) -> Result<(RealtimeSender, RealtimeReceiver), SpeechError> {
    let auth = crate::auth::bearer(key).ok_or(SpeechError::MalformedKey)?;
    let socket = crate::ws::connect(&realtime_url(api_base), auth, CONNECT_TIMEOUT).await?;
    let (sink, mut stream) = socket.split();
    let created = tokio::time::timeout(CONNECT_TIMEOUT, async {
        while let Some(message) = stream.next().await {
            let Ok(Message::Text(text)) = message else {
                if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                    break;
                }
                continue;
            };
            let value: Value = serde_json::from_str(text.as_str()).unwrap_or_default();
            match value.get("type").and_then(Value::as_str) {
                Some("session.created") => return Ok(()),
                Some("error") => return Err(SpeechError::Network(error_message(&value))),
                _ => {}
            }
        }
        Err(SpeechError::Network(
            "the live transcription closed before it started".to_owned(),
        ))
    })
    .await
    .map_err(|_| SpeechError::Network("the live transcription did not start in time".to_owned()))?;
    created?;
    let mut sender = RealtimeSender { sink };
    sender.send(format_message()).await?;
    Ok((sender, RealtimeReceiver { stream }))
}

impl RealtimeSender {
    async fn send(&mut self, text: String) -> Result<(), SpeechError> {
        self.sink
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| SpeechError::Network("the live transcription connection broke".to_owned()))
    }

    /// Sends 16 kHz `pcm_s16le` bytes (split into messages of at most 262,144 bytes).
    ///
    /// # Errors
    /// [`SpeechError::Network`] when the connection broke.
    pub async fn append(&mut self, pcm: &[u8]) -> Result<(), SpeechError> {
        for part in pcm.chunks(MAX_CHUNK_BYTES) {
            let audio = base64::engine::general_purpose::STANDARD.encode(part);
            self.send(json!({"type": "input_audio.append", "audio": audio}).to_string())
                .await?;
        }
        Ok(())
    }

    /// Ends the audio: `input_audio.flush` then `input_audio.end`; the final text follows on the receiver.
    ///
    /// # Errors
    /// [`SpeechError::Network`] when the connection broke.
    pub async fn finish(&mut self) -> Result<(), SpeechError> {
        self.send(json!({"type": "input_audio.flush"}).to_string())
            .await?;
        self.send(json!({"type": "input_audio.end"}).to_string())
            .await
    }

    /// Closes the connection (cancel, or after the final text).
    pub async fn close(mut self) {
        let _ = self.sink.send(Message::Close(None)).await;
        let _ = self.sink.close().await;
    }
}

impl RealtimeReceiver {
    /// The next event this client uses; [`RealtimeEvent::Closed`] once the connection ends.
    pub async fn next(&mut self) -> RealtimeEvent {
        loop {
            match self.stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Some(event) = parse_event(text.as_str()) {
                        return event;
                    }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => return RealtimeEvent::Closed,
                Some(Ok(_)) => {}
            }
        }
    }
}
