//! Voxtral speech calls of the key service: text-to-speech, the voices list,
//! batch transcription and opening a realtime transcription. Like OCR they go straight to Mistral
//! (`test_api_base`, normally `https://api.mistral.ai`) with the key in use, not through the model proxy.
//!
//! Depends on: vw-mistral (`tts`, `stt_batch`, `realtime`), `service` (`key_for_call`, `api`). Called by:
//! vw-core-api (`speech`, `dictation`).
//! Invariants: one-way (no function returns the key); without a key the unlock or setup prompt is raised
//! and the call fails at once (no waiting, no retry behind the user's back).

use vw_keyproxy::KeyNeed;
use vw_mistral::SpeechError;
use vw_mistral::realtime::{RealtimeReceiver, RealtimeSender};
use vw_mistral::stt_batch::Transcript;
use vw_mistral::tts::{Speech, Voice};

use crate::service::KeyService;

/// Why a speech request did not run or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpeechFailure {
    /// No key is available: the vault is locked or no key is set up (the prompt was raised).
    #[error("{}", match .0 {
        KeyNeed::Unlock => "Unlock your vault to use voice, then try again.",
        KeyNeed::Setup => "Set up your Mistral API key to use voice.",
    })]
    KeyNeeded(KeyNeed),
    /// Mistral answered with an error or could not be reached.
    #[error(transparent)]
    Speech(SpeechError),
}

impl KeyService {
    /// Speaks `text` with `voice` (TTS, WAV).
    ///
    /// # Errors
    /// [`SpeechFailure`].
    pub async fn speak(&self, text: &str, voice: &str) -> Result<Speech, SpeechFailure> {
        let key = self.key_for_call().map_err(SpeechFailure::KeyNeeded)?;
        let (client, base) = self.api();
        vw_mistral::tts::speak(
            client,
            base,
            key.as_str(),
            text,
            voice,
            vw_mistral::tts::TTS_TIMEOUT,
        )
        .await
        .map_err(SpeechFailure::Speech)
    }

    /// The voices the key can use (presets first).
    ///
    /// # Errors
    /// [`SpeechFailure`].
    pub async fn voices(&self) -> Result<Vec<Voice>, SpeechFailure> {
        let key = self.key_for_call().map_err(SpeechFailure::KeyNeeded)?;
        let (client, base) = self.api();
        vw_mistral::tts::voices(client, base, key.as_str())
            .await
            .map_err(SpeechFailure::Speech)
    }

    /// Transcribes one WAV file (batch); `language` an ISO code or `None` for detection.
    ///
    /// # Errors
    /// [`SpeechFailure`].
    pub async fn transcribe(
        &self,
        wav: &[u8],
        language: Option<&str>,
    ) -> Result<Transcript, SpeechFailure> {
        let key = self.key_for_call().map_err(SpeechFailure::KeyNeeded)?;
        let (client, base) = self.api();
        vw_mistral::stt_batch::transcribe(
            client,
            base,
            key.as_str(),
            wav,
            language,
            vw_mistral::stt_batch::BATCH_TIMEOUT,
        )
        .await
        .map_err(SpeechFailure::Speech)
    }

    /// Fails at once with [`SpeechFailure::KeyNeeded`] (raising the prompt) when no key is available; used
    /// before a dictation starts, so the microphone is never opened for nothing.
    ///
    /// # Errors
    /// [`SpeechFailure::KeyNeeded`].
    pub fn speech_ready(&self) -> Result<(), SpeechFailure> {
        self.key_for_call()
            .map(|_| ())
            .map_err(SpeechFailure::KeyNeeded)
    }

    /// Opens a realtime transcription (connected, format set).
    ///
    /// # Errors
    /// [`SpeechFailure`].
    pub async fn realtime(&self) -> Result<(RealtimeSender, RealtimeReceiver), SpeechFailure> {
        let key = self.key_for_call().map_err(SpeechFailure::KeyNeeded)?;
        let (_, base) = self.api();
        vw_mistral::realtime::connect(base, key.as_str())
            .await
            .map_err(SpeechFailure::Speech)
    }
}
