//! The speech part of the command API: the
//! requests both clients send inside `Command::Speech`, what comes back, and the `dictation` event.
//!
//! Depends on: serde, vw-mistral (`Voice`). Called by: `command`, `event`, `speech` (the handler),
//! `dictation`, the UI through the generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - Audio comes in only as PCM chunks of a stream the same origin started; nothing here reads a file.
//! - No request or answer carries the key.

use serde::{Deserialize, Serialize};

/// One speech request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SpeechRequest {
    /// Window receipt after preparing the first speech call, before accepting audio into its queue.
    ReadingReady {
        /// The pending reading event's id.
        request_id: String,
        /// False when speech could not be made; no bar or audio is added.
        available: bool,
        /// True with unavailable when the window already holds two requested readings.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        busy: Option<bool>,
    },
    /// Queue a one-time reading of this session's latest finished reply or the supplied text.
    ReadAloud {
        /// The owner-selected session; session agents use the credential-scoped tool instead.
        session_id: String,
        /// A requested part, or None for the last finished reply.
        text: Option<String>,
    },
    /// Starts a dictation. The window asks the native confirmation (the microphone turns on and what it hears
    /// goes to Mistral); the Debug API's is answered by the core; the UI opens the microphone only after this succeeded.
    Start {
        /// The rate the webview captures at (16,000 preferred; 8,000 to 96,000 accepted).
        sample_rate: u32,
        /// `null`: Auto (live text, realtime); an ISO code (`en`, `fr`, ...): batch on stop with that
        /// language (the realtime model cannot force a language).
        language: Option<String>,
    },
    /// The next audio of a stream: base64 of 16-bit little-endian mono PCM at the start's rate, at most
    /// 256 KB decoded. Only the origin that started the stream may append.
    Append {
        /// The stream.
        stream_id: String,
        /// Base64 PCM.
        pcm: String,
    },
    /// Ends the audio and waits for the text.
    Stop {
        /// The stream.
        stream_id: String,
    },
    /// Drops a stream and its audio (nothing more is sent to Mistral).
    Cancel {
        /// The stream.
        stream_id: String,
    },
    /// Speaks a text with Voxtral TTS (WAV). The window asks the native confirmation; the Debug API's is answered by the core.
    Speak {
        /// At most 2,000 characters.
        text: String,
        /// A voice from `voices` (`null`: the default, `gb_jane_neutral`).
        voice: Option<String>,
    },
    /// The voices the key can use (presets first). Read-only.
    Voices {},
}

/// How a dictation turns speech into text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DictationMode {
    /// Voxtral realtime: live partial text while speaking.
    Realtime,
    /// Voxtral batch: the text when the user stops.
    Batch,
}

/// What a speech request answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SpeechResult {
    /// Whether a one-time reading started, with a plain sentence for the caller.
    Reading {
        /// started, nobodyListening, keyMissing, voiceUnavailable or noReply (accepted/expired for a window receipt).
        status: String,
        /// One plain sentence explaining the outcome.
        message: String,
    },
    /// `start`.
    Started {
        /// The new stream (random).
        stream_id: String,
        /// Realtime (Auto language) or batch (a fixed language).
        mode: DictationMode,
        /// The longest dictation, in seconds.
        max_seconds: u32,
    },
    /// `append`.
    Appended {
        /// Seconds of audio so far.
        seconds: f64,
        /// True once the limit is reached: the UI stops and asks for the text.
        at_limit: bool,
    },
    /// `stop`.
    Transcript {
        /// The text (trimmed; empty when nothing was said).
        text: String,
        /// Which path produced it.
        via: DictationMode,
        /// The model Mistral answered with.
        model: String,
        /// Seconds of audio.
        seconds: f64,
        /// Why the live text was not used, when the batch fallback ran.
        note: Option<String>,
    },
    /// `cancel`.
    Cancelled {
        /// False when there was no such stream.
        cancelled: bool,
    },
    /// `speak`.
    Speech {
        /// `audio/wav`.
        mime: String,
        /// Base64 of the audio file.
        audio: String,
        /// Characters spoken.
        chars: u32,
    },
    /// `voices`.
    Voices {
        /// Presets first, then the user's own.
        voices: Vec<VoiceView>,
    },
}

/// One voice of the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    /// What `speak` takes as `voice`.
    pub voice_id: String,
    /// Display name.
    pub name: String,
    /// Languages (`en_gb`, ...).
    pub languages: Vec<String>,
    /// Gender label, when given.
    pub gender: Option<String>,
    /// Mistral's preset (true) or the user's own voice.
    pub preset: bool,
}

impl From<vw_mistral::Voice> for VoiceView {
    fn from(v: vw_mistral::Voice) -> Self {
        Self {
            voice_id: v.voice_id,
            name: v.name,
            languages: v.languages,
            gender: v.gender,
            preset: v.preset,
        }
    }
}

/// What happened in a running dictation (the `dictation` event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DictationUpdate {
    /// The live text so far (the whole text, not a piece: a client that missed one still shows the truth).
    Partial {
        /// Text so far.
        text: String,
    },
    /// Live text stopped working; the text comes from the batch path when the user stops.
    LiveLost {
        /// Why (Mistral's sentence or a network reason).
        message: String,
    },
    /// The stream ended without a `stop` (no audio for a while, or the core shut it).
    Ended {
        /// Why.
        reason: String,
    },
}
