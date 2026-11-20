//! Firefox and Safari video uses the app encoder with PNGs from the helper's guarded screenshot action.
//! Each sample checks the original chat, helper session and current mode; no page-supplied pixels enter the encoder.
use super::external_video_file::{prepare_name, save, valid_filename};
#[path = "external_video_flow.rs"]
mod flow;
use super::{owner, read_consent::Consent};
use crate::core::Shared;
use base64::Engine as _;
use flow::{start, stop};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio::task::JoinHandle;
use vw_browser::recording::{VideoBridge, VideoCommand, VideoReply};
#[path = "external_video_frame.rs"]
mod frame_capture;
use frame_capture::{alive, complete, frame, identity};

const INTERVAL: Duration = Duration::from_millis(250);
const MAX_FRAMES: usize = 1200;
const MAX_PNG: usize = 2 * 1024 * 1024;
const MAX_WEBM: usize = 32 * 1024 * 1024;
/// Start refusal when the first guarded frame shows a protected value (video never records one, also in Full).
pub(crate) const PROTECTED_START: &str = "This page shows a password, PIN or card value, and video never records those. Start the recording on a page without one, or use an action GIF.";
const ID_FIELDS: &[&str] = &[
    "profileId",
    "browserSessionId",
    "leaseEpoch",
    "policyRevision",
    "documentId",
    "tabId",
    "recordingId",
];

#[derive(Default)]
pub(crate) struct Videos {
    bridge: RwLock<Option<Arc<dyn VideoBridge>>>,
    active: Mutex<HashMap<String, Active>>,
    starting: Arc<Mutex<HashSet<String>>>,
    operations: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
    #[cfg(test)]
    operation_calls: std::sync::atomic::AtomicUsize,
}
struct Active {
    chat: String,
    start: Value,
    mode: String,
    started: Instant,
    max_seconds: u64,
    cancel: watch::Sender<u8>,
    worker: JoinHandle<Stats>,
}
struct Capture {
    chat: String,
    session: String,
    identity: Value,
    mode: String,
    started: Instant,
    max_seconds: u64,
    bridge: Arc<dyn VideoBridge>,
}
#[derive(Default)]
struct Stats {
    frames: usize,
    omitted: usize,
    failed: bool,
    limit: Option<&'static str>,
}
enum Frame {
    Added,
    Omitted,
    Protected,
}
struct Reservation(Arc<Mutex<HashSet<String>>>, String);
impl Drop for Reservation {
    fn drop(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.1);
    }
}
impl Videos {
    pub(crate) fn operation(&self, session: &str) -> Arc<AsyncMutex<()>> {
        #[cfg(test)]
        self.operation_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut operations = self
            .operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        operations.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = operations.get(session).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(AsyncMutex::new(()));
        operations.insert(session.to_owned(), Arc::downgrade(&lock));
        lock
    }
    #[cfg(test)]
    pub(crate) fn operation_calls(&self) -> usize {
        self.operation_calls
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    pub(crate) fn set_bridge(&self, bridge: Arc<dyn VideoBridge>) {
        *self.bridge.write().unwrap_or_else(PoisonError::into_inner) = Some(bridge);
    }
    pub(crate) fn cancel_chat(&self, chat: &str) {
        let mut finished = Vec::new();
        {
            let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
            active.retain(|_, video| {
                if video.chat != chat {
                    return true;
                }
                video.cancel.send_replace(2);
                if video.worker.is_finished() {
                    if let Some(id) = video.start["recordingId"].as_str() {
                        finished.push(id.to_owned());
                    }
                    false
                } else {
                    true
                }
            });
        }
        if let (Some(bridge), Ok(runtime)) = (
            self.bridge
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            tokio::runtime::Handle::try_current(),
        ) {
            for recording in finished {
                runtime.spawn({
                    let bridge = bridge.clone();
                    async move {
                        cancel_encoder(&bridge, &recording).await;
                    }
                });
            }
        }
    }
    fn bridge(&self) -> Result<Arc<dyn VideoBridge>, String> {
        self.bridge
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                "The app's video encoder is unavailable; use an action GIF instead.".into()
            })
    }
    fn reserve(&self, session: &str) -> Result<Reservation, String> {
        let mut starting = self.starting.lock().unwrap_or_else(PoisonError::into_inner);
        if self
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(session)
            || !starting.insert(session.to_owned())
        {
            return Err(vw_vocabulary::texts::BROWSER_VIDEO_ALREADY_ACTIVE.into());
        }
        Ok(Reservation(self.starting.clone(), session.to_owned()))
    }
    fn take_for_stop(
        &self,
        session: &str,
        chat: &str,
        tab: &Value,
    ) -> Result<(Active, Reservation), String> {
        let mut starting = self.starting.lock().unwrap_or_else(PoisonError::into_inner);
        if starting.contains(session) {
            return Err(vw_vocabulary::texts::BROWSER_VIDEO_ALREADY_ACTIVE.into());
        }
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        let current = active
            .get(session)
            .ok_or(vw_vocabulary::texts::BROWSER_VIDEO_RECORDING_MISSING)?;
        if current.chat != chat || (!tab.is_null() && current.start["tabId"] != *tab) {
            return Err("This recording belongs to another chat or tab.".into());
        }
        starting.insert(session.to_owned());
        let video = active
            .remove(session)
            .ok_or(vw_vocabulary::texts::BROWSER_VIDEO_RECORDING_MISSING)?;
        Ok((
            video,
            Reservation(self.starting.clone(), session.to_owned()),
        ))
    }
    fn reserve_failure_cleanup(&self, session: &str, recording: &str) -> Option<Reservation> {
        let mut starting = self.starting.lock().unwrap_or_else(PoisonError::into_inner);
        let active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if starting.contains(session) || active.get(session)?.start["recordingId"] != recording {
            return None;
        }
        starting.insert(session.to_owned());
        Some(Reservation(self.starting.clone(), session.to_owned()))
    }
}

/// The helper's advertised guarded-frame route is limited to the exact current Firefox/Safari session.
pub(super) fn supports(status: &Value, request: &Value) -> bool {
    let body = &status["structuredContent"];
    let profile = body["sessions"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["sessionId"] == request["sessionId"])
        })
        .and_then(|row| row["profileId"].as_str());
    profile.is_some_and(|id| id.starts_with("firefox:") || id.starts_with("safari:"))
        && body["profiles"].as_array().is_some_and(|rows| {
            rows.iter().any(|row| {
                row["profileId"].as_str() == profile
                    && row["connected"] == true
                    && row["extensionCapabilities"]
                        .as_array()
                        .is_some_and(|caps| caps.contains(&json!("page.video.frames.v1")))
                    && row["features"]
                        .as_array()
                        .is_some_and(|names| names.contains(&request["action"]))
            })
        })
}

pub(super) async fn send(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<owner::Answer, String> {
    if request["action"] == "recording.video.start" {
        start(shared, chat, request, consent).await
    } else {
        stop(shared, chat, request, consent).await
    }
}

async fn sample(shared: Weak<Shared>, capture: Capture, mut cancel: watch::Receiver<u8>) -> Stats {
    let mut stats = Stats {
        frames: 1,
        ..Stats::default()
    };
    let recording = capture.identity["recordingId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut cadence = tokio::time::interval(INTERVAL);
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    cadence.tick().await;
    loop {
        tokio::select! { biased;
            _ = cancel.changed() => { stats.failed = *cancel.borrow() == 2; break; }
            _ = cadence.tick() => {}
        }
        if *cancel.borrow() != 0 {
            stats.failed = *cancel.borrow() == 2;
            break;
        }
        if stats.frames >= MAX_FRAMES {
            stats.limit = Some("frames");
            break;
        }
        if capture.started.elapsed() >= Duration::from_secs(capture.max_seconds) {
            stats.limit = Some("duration");
            break;
        }
        let Some(shared) = shared.upgrade() else {
            stats.failed = true;
            break;
        };
        match frame(&shared, &capture).await {
            Ok(Frame::Added) => stats.frames += 1,
            Ok(Frame::Omitted) => stats.omitted += 1,
            Ok(Frame::Protected) => {
                stats.limit = Some("protected_field");
                break;
            }
            Err(reason) => {
                // Fixed app-side reason strings only; never page content.
                tracing::warn!(%reason, frames = stats.frames, "browser video sampling ended");
                stats.failed = true;
                break;
            }
        }
    }
    if stats.failed {
        cancel_encoder(&capture.bridge, &recording).await;
        if let Some(shared) = shared.upgrade() {
            let reservation = shared
                .chrome_control
                .external_video
                .reserve_failure_cleanup(&capture.session, &recording);
            let Some(_cleanup) = reservation else {
                return stats;
            };
            let request =
                json!({"sessionId":capture.session,"args":{"tabId":capture.identity["tabId"]}});
            discard_helper(&shared, &capture.chat, &request).await;
            {
                let mut active = shared
                    .chrome_control
                    .external_video
                    .active
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if active
                    .get(&capture.session)
                    .is_some_and(|current| current.start["recordingId"] == recording)
                {
                    active.remove(&capture.session);
                }
            }
        }
    }
    stats
}

/// The helper's session/profile rows name Safari as `safari:<uuid>`, while Safari's extension stamps its own bare
/// UUID into recording and capture identities. Matches exactly, or exactly that one namespace mapping.
pub(crate) fn same_profile(reported: &Value, row: &Value) -> bool {
    reported == row
        || reported.as_str().is_some_and(|raw| {
            !raw.is_empty() && !raw.contains(':') && row.as_str() == Some(&format!("safari:{raw}"))
        })
}
async fn cancel_encoder(bridge: &Arc<dyn VideoBridge>, recording: &str) {
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        bridge.send(VideoCommand::Cancel {
            recording: recording.to_owned(),
        }),
    )
    .await;
}
async fn discard_helper(shared: &Arc<Shared>, chat: &str, request: &Value) {
    let mut args = json!({"discard":true});
    if !request["args"]["tabId"].is_null() {
        args["tabId"] = request["args"]["tabId"].clone();
    }
    let stop =
        json!({"sessionId":request["sessionId"],"action":"recording.video.stop","args":args});
    let _ = super::escalation::send(shared, chat, "chrome_execute", stop, None).await;
}
