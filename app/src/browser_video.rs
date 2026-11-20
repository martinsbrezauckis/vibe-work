//! Main-webview-only `WebCodecs` bridge for guarded native browser screenshots.
//! The browser retains capture authority; this channel only encodes bytes already admitted by its guards.
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tauri::ipc::Channel;
use tauri::{State, Webview};
use tokio::sync::oneshot;
use vw_browser::recording::{VideoBridge, VideoCommand, VideoFuture, VideoReply};

#[derive(Clone, Default)]
/// One process-local request channel and its pending main-webview replies.
pub struct BrowserVideo(Arc<Inner>);

#[derive(Default)]
struct Inner {
    next: AtomicU64,
    requests: Mutex<Connection>,
    pending: Mutex<HashMap<u64, oneshot::Sender<UiReply>>>,
}

#[derive(Default)]
struct Connection {
    epoch: u64,
    channel: Option<Channel<UiRequest>>,
    recordings: HashSet<String>,
}

const CLOSED: &str = "The app's video encoder closed; use an action GIF instead.";

struct PendingGuard(Arc<Inner>, u64);
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.0
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.1);
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// A bounded encoder request delivered only to the main app webview.
pub struct UiRequest {
    id: u64,
    recording: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    png: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    at_seconds: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
/// The main app webview's answer to one pending encoder request.
pub struct UiReply {
    id: u64,
    ok: bool,
    supported: Option<bool>,
    bytes: Option<String>,
    frames: Option<usize>,
    duration_ms: Option<u64>,
    error: Option<String>,
}

/// Installs a new main-document channel and drops replies to the replaced document.
/// Replaces the request channel after a main document load.
pub fn subscribe(
    webview: &Webview,
    state: &State<'_, BrowserVideo>,
    requests: Channel<UiRequest>,
) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("Only the app webview can encode browser video".into());
    }
    state.replace_channel(requests);
    Ok(())
}

impl BrowserVideo {
    fn replace_channel(&self, requests: Channel<UiRequest>) {
        let mut connection = self
            .0
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        connection.epoch = connection.epoch.wrapping_add(1);
        connection.recordings.clear();
        self.0
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        connection.channel = Some(requests);
    }
}

/// Completes exactly one request issued to this main document.
/// Resolves a pending request by its exact id.
pub fn reply(
    webview: &Webview,
    state: &State<'_, BrowserVideo>,
    reply: UiReply,
) -> Result<bool, String> {
    if webview.label() != "main" {
        return Err("Only the app webview can encode browser video".into());
    }
    let pending = state
        .0
        .pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&reply.id);
    Ok(pending.is_some_and(|sender| sender.send(reply).is_ok()))
}

impl VideoBridge for BrowserVideo {
    fn send(&self, command: VideoCommand) -> VideoFuture<'_> {
        Box::pin(async move {
            let id = self.0.next.fetch_add(1, Ordering::Relaxed) + 1;
            let (recording, kind, png, at_seconds) = split_command(command);
            let (sender, receiver) = oneshot::channel();
            let _pending = PendingGuard(self.0.clone(), id);
            let request = UiRequest {
                id,
                recording: recording.clone(),
                kind,
                png,
                at_seconds,
            };
            let (epoch, delivered) = {
                let mut connection = self
                    .0
                    .requests
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if kind == "start" {
                    if !connection.recordings.insert(recording.clone()) {
                        return Err("Page video encoding failed".into());
                    }
                } else if !connection.recordings.contains(&recording) {
                    return if kind == "cancel" {
                        Ok(VideoReply::Added)
                    } else {
                        Err(CLOSED.into())
                    };
                }
                self.0
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(id, sender);
                let delivered = connection
                    .channel
                    .as_ref()
                    .is_some_and(|channel| channel.send(request).is_ok());
                if !delivered {
                    connection.recordings.remove(&recording);
                }
                (connection.epoch, delivered)
            };
            if !delivered {
                return Err(
                    "The app's video encoder is unavailable; use an action GIF instead.".into(),
                );
            }
            let answer = receiver.await.map_err(|_| CLOSED.to_owned())?;
            {
                let mut connection = self
                    .0
                    .requests
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if connection.epoch != epoch || !connection.recordings.contains(&recording) {
                    return Err(CLOSED.into());
                }
                if kind == "cancel"
                    || (kind == "finish" && answer.ok)
                    || (kind == "start" && (!answer.ok || answer.supported == Some(false)))
                {
                    connection.recordings.remove(&recording);
                }
            }
            if answer.supported == Some(false) {
                return Ok(VideoReply::Unsupported);
            }
            if !answer.ok {
                return Err(answer
                    .error
                    .unwrap_or_else(|| "Page video encoding failed".into()));
            }
            match kind {
                "start" => Ok(VideoReply::Ready),
                "frame" | "cancel" => Ok(VideoReply::Added),
                _ => finished(answer),
            }
        })
    }

    fn capture_allowed(&self, _: &str) -> bool {
        true
    }
}

/// Splits one bridge command into the wire fields sent to the main webview.
fn split_command(command: VideoCommand) -> (String, &'static str, Option<String>, Option<f64>) {
    match command {
        VideoCommand::Start { recording } => (recording, "start", None, None),
        VideoCommand::Frame {
            recording,
            png,
            at_seconds,
        } => (
            recording,
            "frame",
            Some(base64::engine::general_purpose::STANDARD.encode(png)),
            Some(at_seconds),
        ),
        VideoCommand::Finish {
            recording,
            at_seconds,
        } => (recording, "finish", None, Some(at_seconds)),
        VideoCommand::Cancel { recording } => (recording, "cancel", None, None),
    }
}

/// Decodes a successful Finish answer; the 45 MiB base64 bound is checked before decoding.
fn finished(answer: UiReply) -> Result<VideoReply, String> {
    let encoded = answer.bytes.ok_or("The page video has no encoded bytes")?;
    if encoded.len() > 45 * 1024 * 1024 {
        return Err("The page video is too large".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "The page video bytes are invalid".to_owned())?;
    Ok(VideoReply::Finished {
        bytes,
        frames: answer.frames.ok_or("The page video has no frame count")?,
        duration_ms: answer.duration_ms.ok_or("The page video has no duration")?,
    })
}

#[cfg(test)]
#[path = "browser_video_tests.rs"]
mod tests;
