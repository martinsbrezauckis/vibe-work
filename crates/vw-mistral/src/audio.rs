//! Audio helpers for dictation: the webview sends 16-bit little-endian mono PCM; Voxtral
//! realtime takes `pcm_s16le` at 16 kHz and the batch endpoint takes a WAV file. This module checks the
//! chunks, converts other capture rates to 16 kHz (a stateful resampler, so chunk borders leave no
//! clicks), and wraps PCM into a WAV file.
//!
//! Depends on: nothing. Called by: `realtime`, `stt_batch`, vw-core-api (`dictation`).
//! Invariants: limits are checked before anything is stored; no function panics on any input.

/// The rate Voxtral realtime is fed with (the SDK's and Vibe's choice: `pcm_s16le` at 16 kHz).
pub const STT_SAMPLE_RATE: u32 = 16_000;

/// Largest decoded audio in one realtime `input_audio.append` (Mistral SDK:
/// `RealtimeTranscriptionInputAudioAppend.audio`, "Max decoded size: 262144 bytes").
pub const MAX_CHUNK_BYTES: usize = 262_144;

/// Lowest and highest capture rates accepted from the webview (8 kHz telephone to 96 kHz studio).
pub const RATE_RANGE: std::ops::RangeInclusive<u32> = 8_000..=96_000;

/// Why audio was refused. Texts are shown to the user as they are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    /// A chunk with an odd number of bytes cannot be 16-bit samples.
    #[error("the audio chunk is not 16-bit samples (odd length)")]
    OddLength,
    /// A chunk larger than [`MAX_CHUNK_BYTES`].
    #[error("an audio chunk was larger than 256 KB")]
    ChunkTooLarge,
    /// A capture rate outside [`RATE_RANGE`].
    #[error("the microphone rate {0} Hz is not supported (8,000 to 96,000 Hz)")]
    Rate(u32),
}

/// Reads 16-bit little-endian samples.
///
/// # Errors
/// [`AudioError::OddLength`], [`AudioError::ChunkTooLarge`].
pub fn pcm16_from_le(bytes: &[u8]) -> Result<Vec<i16>, AudioError> {
    if bytes.len() > MAX_CHUNK_BYTES {
        return Err(AudioError::ChunkTooLarge);
    }
    if !bytes.len().is_multiple_of(2) {
        return Err(AudioError::OddLength);
    }
    Ok(bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect())
}

/// Writes samples as 16-bit little-endian bytes.
#[must_use]
pub fn pcm16_to_le(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// A WAV file (RIFF, PCM format 1, mono, 16-bit) holding `samples` at `rate`.
#[must_use]
pub fn wav_pcm16(samples: &[i16], rate: u32) -> Vec<u8> {
    // Sizes saturate instead of overflowing; dictation is capped far below 4 GB anyway.
    let data_len = u32::try_from(samples.len().saturating_mul(2)).unwrap_or(u32::MAX - 36);
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&pcm16_to_le(samples));
    out
}

/// Converts a stream of chunks from one capture rate to [`STT_SAMPLE_RATE`]. Keeps its position and the
/// last input samples between chunks, so the output equals what one call over the whole clip would give.
///
/// Method: a moving average over one output period (a cheap low-pass against aliasing when the input is
/// faster), then linear interpolation at the output instants. Good enough for speech recognition; not a
/// studio resampler.
#[derive(Debug, Clone)]
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Width of the moving average in input samples (1 = none).
    window: usize,
    /// Position of the next output sample, in input samples counted from `history[0]`.
    position: f64,
    /// Filtered input samples not yet fully used (the last one or two are needed by the next chunk).
    history: Vec<f64>,
    /// Raw input kept for the moving average across chunk borders.
    tail: Vec<f64>,
}

impl Resampler {
    /// A resampler from `rate` Hz to 16 kHz.
    ///
    /// # Errors
    /// [`AudioError::Rate`] outside [`RATE_RANGE`].
    pub fn new(rate: u32) -> Result<Self, AudioError> {
        if !RATE_RANGE.contains(&rate) {
            return Err(AudioError::Rate(rate));
        }
        let step = f64::from(rate) / f64::from(STT_SAMPLE_RATE);
        // Rates up to 96 kHz give a step of at most 6, so the cast is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let window = if step > 1.0 { step.round() as usize } else { 1 };
        Ok(Self {
            step,
            window,
            position: 0.0,
            history: Vec::new(),
            tail: Vec::new(),
        })
    }

    /// True when no conversion happens (the capture is already 16 kHz).
    #[must_use]
    pub fn is_identity(&self) -> bool {
        (self.step - 1.0).abs() < f64::EPSILON
    }

    /// Converts the next chunk.
    #[must_use]
    pub fn push(&mut self, input: &[i16]) -> Vec<i16> {
        if self.is_identity() {
            return input.to_vec();
        }
        // Moving average over `window` raw samples (causal; the half-window delay is irrelevant here).
        for &sample in input {
            self.tail.push(f64::from(sample));
            if self.tail.len() > self.window {
                self.tail.remove(0);
            }
            #[allow(clippy::cast_precision_loss)]
            let mean = self.tail.iter().sum::<f64>() / self.tail.len() as f64;
            self.history.push(mean);
        }
        let mut out = Vec::new();
        #[allow(clippy::cast_precision_loss)]
        while self.position + 1.0 < self.history.len() as f64 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i = self.position.floor() as usize;
            let frac = self.position - self.position.floor();
            let value = self.history[i] * (1.0 - frac) + self.history[i + 1] * frac;
            #[allow(clippy::cast_possible_truncation)]
            out.push(value.round().clamp(-32_768.0, 32_767.0) as i16);
            self.position += self.step;
        }
        // Drop what no later output needs (keep the sample under the current position).
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let used = (self.position.floor() as usize).min(self.history.len());
        self.history.drain(..used);
        #[allow(clippy::cast_precision_loss)]
        {
            self.position -= used as f64;
        }
        out
    }
}

/// Seconds of 16 kHz audio in `samples` samples.
#[must_use]
pub fn seconds_at_16k(samples: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = samples as f64;
    n / f64::from(STT_SAMPLE_RATE)
}
