//! Running dictations: one stream per
//! microphone session, its audio, and the task that feeds Voxtral realtime.
//!
//! A stream keeps every sample (16 kHz, after resampling) so the batch path can run on stop: always for a
//! fixed language, and as the fallback when the realtime connection failed, broke or fell behind. In
//! realtime mode a task connects at once; audio waits in a bounded queue until the session started; live
//! text goes out as `dictation` events carrying the whole text so far.
//!
//! Depends on: vw-mistral (`audio`, `realtime`), vw-keys (`KeyService::realtime`), `speech_view`, `event`,
//! `command` (`Origin`). Called by: `speech` (the handler).
//! Invariants:
//! - Either origin may append to, stop or cancel any stream (the Debug API acts as the user; before,
//!   a stream belonged to the origin that started it). The window goes on with a dictation a Debug API request
//!   started once that request was answered. The starting origin is logged. Ids are random 128-bit values.
//! - At most [`SpeechConfig::max_seconds`] of audio per stream; a stream without audio for
//!   [`SpeechConfig::idle`] after its first audio, or without any audio for [`SpeechConfig::first_audio_wait`]
//!   (the OS microphone prompt may wait for the user), is dropped and an `ended` event says so (a closed
//!   window cannot leave a stream or a Mistral connection behind).
//! - Every task is owned (its handle is aborted when the stream goes, and when the registry drops).
//! - Logs carry stream ids, sizes and outcomes; never audio, text or the key.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vw_mistral::audio::{Resampler, pcm16_to_le, seconds_at_16k};
use vw_mistral::realtime::RealtimeEvent;

use crate::command::Origin;
use crate::event::CoreEvent;
use crate::speech_view::{DictationMode, DictationUpdate};

/// Dictation and speech settings.
#[derive(Debug, Clone)]
pub struct SpeechConfig {
    /// Longest dictation in seconds (default 300: five minutes).
    pub max_seconds: u32,
    /// A stream with no audio for this long, once its first audio arrived, is dropped (default 30 s).
    pub idle: Duration,
    /// How long a stream may wait for its FIRST audio (default 10 minutes). The microphone opens after
    /// the core started the stream, and on macOS the first use waits for the user's answer to the OS prompt, so
    /// `idle` must not run then (it ended first dictations whose prompt took over 30 s). This cap only keeps a
    /// window that closed while it waited from leaving a stream and a Mistral connection behind.
    pub first_audio_wait: Duration,
    /// Longest wait for the realtime final text after `stop` (default 20 s).
    pub stop_wait: Duration,
}

impl Default for SpeechConfig {
    fn default() -> Self {
        Self {
            max_seconds: 300,
            idle: Duration::from_secs(30),
            first_audio_wait: Duration::from_secs(600),
            stop_wait: Duration::from_secs(20),
        }
    }
}

/// Audio queued for the realtime task before the connection is up (250 ms chunks: ~8 minutes).
const QUEUE: usize = 2_048;

/// A message to the realtime task.
enum Feed {
    /// 16 kHz `pcm_s16le` bytes.
    Audio(Vec<u8>),
    /// No more audio: flush, end and report the final text.
    Finish,
}

/// How the realtime part of a stream ended.
pub(crate) type LiveOutcome = Result<(String, String), String>;

/// One running dictation.
pub(crate) struct Stream {
    pub(crate) mode: DictationMode,
    pub(crate) language: Option<String>,
    resampler: Resampler,
    pub(crate) samples: Vec<i16>,
    last_audio: Instant,
    /// True once the first audio arrived (the microphone is open): from then on `idle` applies.
    heard: bool,
    feed: Option<mpsc::Sender<Feed>>,
    /// Why live text stopped, once it did (the batch path runs on stop).
    live_lost: Option<String>,
    outcome: Option<oneshot::Receiver<LiveOutcome>>,
    task: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        for task in [self.task.take(), self.watchdog.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
    }
}

/// What `take_for_stop` hands the handler.
pub(crate) struct Stopped {
    pub(crate) mode: DictationMode,
    pub(crate) language: Option<String>,
    pub(crate) samples: Vec<i16>,
    /// The realtime result, when realtime was still alive (wait on it with `stop_wait`).
    pub(crate) live: Option<oneshot::Receiver<LiveOutcome>>,
    pub(crate) live_lost: Option<String>,
    /// The realtime task, still running until it reported (the handler aborts it after waiting).
    pub(crate) task: Option<JoinHandle<()>>,
}

/// Why a stream operation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamError {
    /// No such stream (never started, stopped, cancelled or dropped as idle).
    Unknown,
}

/// The registry of running dictations.
pub(crate) struct Dictations {
    streams: Mutex<HashMap<String, Stream>>,
    events: broadcast::Sender<CoreEvent>,
    config: SpeechConfig,
    me: Weak<Dictations>,
}

fn random_id() -> String {
    use std::fmt::Write as _;
    let mut buf = [0_u8; 16];
    // Ids are not a permission (either origin may act on any stream); without an OS random source the time keeps
    // them unique.
    if getrandom::fill(&mut buf).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        buf = nanos.to_le_bytes();
    }
    buf.iter().fold(String::from("dict-"), |mut id, b| {
        let _ = write!(id, "{b:02x}");
        id
    })
}

impl Dictations {
    pub(crate) fn new(events: broadcast::Sender<CoreEvent>, config: SpeechConfig) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            streams: Mutex::new(HashMap::new()),
            events,
            config,
            me: me.clone(),
        })
    }

    pub(crate) fn config(&self) -> &SpeechConfig {
        &self.config
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Stream>> {
        self.streams.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, stream_id: &str, update: DictationUpdate) {
        let _ = self.events.send(CoreEvent::Dictation {
            stream_id: stream_id.to_owned(),
            update,
        });
    }

    /// Registers a new stream; in realtime mode starts its task with `keys`.
    pub(crate) fn start(
        &self,
        origin: Origin,
        resampler: Resampler,
        language: Option<String>,
        keys: Option<Arc<vw_keys::KeyService>>,
    ) -> (String, DictationMode) {
        let id = random_id();
        let mode = if language.is_none() && keys.is_some() {
            DictationMode::Realtime
        } else {
            DictationMode::Batch
        };
        let (feed, task, outcome) = match (mode, keys) {
            (DictationMode::Realtime, Some(keys)) => {
                let (tx, rx) = mpsc::channel(QUEUE);
                let (done_tx, done_rx) = oneshot::channel();
                let task = tokio::spawn(run_live(self.me.clone(), id.clone(), keys, rx, done_tx));
                (Some(tx), Some(task), Some(done_rx))
            }
            _ => (None, None, None),
        };
        let watchdog = self.watchdog(id.clone());
        self.lock().insert(
            id.clone(),
            Stream {
                mode,
                language,
                resampler,
                samples: Vec::new(),
                last_audio: Instant::now(),
                heard: false,
                feed,
                live_lost: None,
                outcome,
                task,
                watchdog: Some(watchdog),
            },
        );
        info!(stream = %id, ?mode, origin = origin.as_str(), "dictation started");
        (id, mode)
    }

    /// A task that drops the stream once it has had no audio for `idle` after its first audio, or no audio at all
    /// for `first_audio_wait` (the microphone never opened:, see [`SpeechConfig::first_audio_wait`]).
    fn watchdog(&self, id: String) -> JoinHandle<()> {
        let me = self.me.clone();
        let (idle, first) = (self.config.idle, self.config.first_audio_wait);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250).min(idle)).await;
                let Some(me) = me.upgrade() else { return };
                let expired = {
                    let mut streams = me.lock();
                    let limit = match streams.get(&id) {
                        None => return,
                        Some(s) if s.heard => idle,
                        Some(_) => first,
                    };
                    match streams.get(&id) {
                        Some(s) if s.last_audio.elapsed() >= limit => {
                            streams.remove(&id).map(|s| s.heard)
                        }
                        _ => None,
                    }
                };
                if let Some(heard) = expired {
                    warn!(stream = %id, heard, "dictation dropped: no audio");
                    let reason = if heard {
                        format!(
                            "No audio arrived for {} seconds, so the dictation was stopped.",
                            idle.as_secs()
                        )
                    } else {
                        format!(
                            "The microphone did not open within {} minutes, so the dictation was stopped.",
                            first.as_secs() / 60
                        )
                    };
                    me.emit(&id, DictationUpdate::Ended { reason });
                    return;
                }
            }
        })
    }

    /// Adds PCM (at the stream's capture rate) to a stream; returns the seconds so far and whether the
    /// limit is reached (audio past the limit is dropped).
    pub(crate) fn append(&self, id: &str, pcm: &[i16]) -> Result<(f64, bool), StreamError> {
        let max = usize::try_from(self.config.max_seconds).unwrap_or(usize::MAX) * 16_000;
        let mut lost = None;
        let result = {
            let mut streams = self.lock();
            let stream = streams.get_mut(id).ok_or(StreamError::Unknown)?;
            stream.last_audio = Instant::now();
            stream.heard = true;
            let mut converted = stream.resampler.push(pcm);
            let room = max.saturating_sub(stream.samples.len());
            converted.truncate(room);
            if !converted.is_empty() {
                if let Some(feed) = &stream.feed
                    && feed.try_send(Feed::Audio(pcm16_to_le(&converted))).is_err()
                {
                    // The live connection fell behind or ended: the batch path takes over on stop.
                    stream.feed = None;
                    if stream.live_lost.is_none() {
                        let why = "Live text fell behind; the text comes when you stop.".to_owned();
                        stream.live_lost = Some(why.clone());
                        lost = Some(why);
                    }
                }
                stream.samples.extend_from_slice(&converted);
            }
            let total = stream.samples.len();
            (seconds_at_16k(total), total >= max)
        };
        if let Some(message) = lost {
            self.emit(id, DictationUpdate::LiveLost { message });
        }
        Ok(result)
    }

    /// Removes a stream for `stop`, telling its realtime task to finish.
    pub(crate) fn take_for_stop(&self, id: &str) -> Result<Stopped, StreamError> {
        let mut stream = self.lock().remove(id).ok_or(StreamError::Unknown)?;
        let live = match stream.feed.take() {
            Some(feed) if feed.try_send(Feed::Finish).is_ok() => stream.outcome.take(),
            _ => None,
        };
        // The task must outlive the stream until the outcome arrives: the handler owns it from here.
        let task = stream.task.take();
        Ok(Stopped {
            mode: stream.mode,
            language: stream.language.take(),
            samples: std::mem::take(&mut stream.samples),
            live,
            live_lost: stream.live_lost.take(),
            task,
        })
    }

    /// Drops a stream and its audio (its task is aborted, closing the connection).
    pub(crate) fn cancel(&self, id: &str) -> bool {
        let cancelled = self.lock().remove(id).is_some();
        if cancelled {
            info!(stream = %id, "dictation cancelled");
        }
        cancelled
    }

    /// Notes that live text stopped (connection failure or Mistral's error) and tells the clients.
    fn lose_live(&self, id: &str, message: String) {
        let first = {
            let mut streams = self.lock();
            match streams.get_mut(id) {
                Some(s) if s.live_lost.is_none() => {
                    s.live_lost = Some(message.clone());
                    s.feed = None;
                    true
                }
                _ => false,
            }
        };
        if first {
            self.emit(id, DictationUpdate::LiveLost { message });
        }
    }
}

/// The realtime task of one stream: connect, forward audio, publish the live text, report the outcome.
async fn run_live(
    registry: Weak<Dictations>,
    id: String,
    keys: Arc<vw_keys::KeyService>,
    mut feed: mpsc::Receiver<Feed>,
    done: oneshot::Sender<LiveOutcome>,
) {
    let lost = |message: String| {
        if let Some(r) = registry.upgrade() {
            r.lose_live(&id, message);
        }
    };
    let (mut sender, mut receiver) = match keys.realtime().await {
        Ok(pair) => pair,
        Err(error) => {
            warn!(stream = %id, %error, "live transcription did not connect");
            let message = error.to_string();
            lost(message.clone());
            let _ = done.send(Err(message));
            return;
        }
    };
    info!(stream = %id, "live transcription connected");
    let mut text = String::new();
    let mut finishing = false;
    let outcome: LiveOutcome = loop {
        tokio::select! {
            next = feed.recv(), if !finishing => match next {
                Some(Feed::Audio(bytes)) => {
                    if let Err(error) = sender.append(&bytes).await {
                        break Err(error.to_string());
                    }
                }
                Some(Feed::Finish) => {
                    finishing = true;
                    if let Err(error) = sender.finish().await {
                        break Err(error.to_string());
                    }
                }
                // Cancelled (the stream and its sender were dropped).
                None => break Err("cancelled".to_owned()),
            },
            event = receiver.next() => match event {
                RealtimeEvent::Delta(piece) => {
                    text.push_str(&piece);
                    if let Some(r) = registry.upgrade() {
                        r.emit(&id, DictationUpdate::Partial { text: text.clone() });
                    }
                }
                RealtimeEvent::Done { text: full, model } => {
                    let full = if full.trim().is_empty() { text.clone() } else { full };
                    break Ok((full.trim().to_owned(), model));
                }
                RealtimeEvent::Failed(message) => break Err(message),
                RealtimeEvent::Closed => break Err(
                    "the live transcription closed before it finished".to_owned(),
                ),
            },
        }
    };
    sender.close().await;
    if let Err(message) = &outcome
        && message != "cancelled"
    {
        warn!(stream = %id, chars = message.len(), "live transcription ended with an error");
        lost(message.clone());
    }
    let _ = done.send(outcome);
}
