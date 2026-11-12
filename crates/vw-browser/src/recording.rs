//! Bounded action-screenshot GIFs through the guarded native screenshot path; the core saves their bytes.
//! The action loop collects frames, with explicit omissions and no background capture after Stop.
use crate::driver::{failed, refused};
use crate::{ActError, Actor, Browser};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
const MAX_FRAMES: usize = 30;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_TIME: Duration = Duration::from_secs(120);

/// Named recording action; unsupported video has an explicit result with the GIF alternative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Begin collecting action screenshots.
    Start,
    /// Finish and encode the collected screenshots.
    Stop,
    /// Request continuous video (requires an engine encoder).
    VideoStart,
    /// Finish continuous video (requires an engine encoder).
    VideoStop,
}
impl Request {
    /// Wire name used by the shared permission table.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Start => "recording.start",
            Self::Stop => "recording.stop",
            Self::VideoStart => "recording.video.start",
            Self::VideoStop => "recording.video.stop",
        }
    }
}
pub(crate) struct Frame {
    pub png: Vec<u8>,
    pub delay: u16,
}
struct Active {
    epoch: u64,
    id: u64,
    started: Instant,
    last: Instant,
    frames: Vec<Frame>,
    bytes: usize,
    omitted: usize,
    reason: Option<String>,
}
#[derive(Clone, Default)]
pub(crate) struct Recordings(Arc<Mutex<HashMap<String, Active>>>);
impl Browser {
    pub(crate) async fn record_action(
        &self,
        page: &str,
        action: &crate::Action,
        by: Actor,
        mut result: Result<Value, ActError>,
    ) -> Result<Value, ActError> {
        if !matches!(action, crate::Action::Recording(_))
            && let Ok(receipt) = &mut result
        {
            let recording = self.record_frame(page, by).await;
            if !recording.is_null() {
                receipt["recording"] = recording;
            }
        }
        result
    }
    pub(crate) async fn recording(
        &self,
        page: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        if matches!(request, Request::VideoStart | Request::VideoStop) {
            return Err(refused(
                "This built-in engine has no supported page-video encoder. Use recording.start and recording.stop for a GIF of action screenshots; it contains no audio or continuous video.",
            ));
        }
        let bound = self.bind_read(page, by).await.map_err(refused)?;
        bound.check_read_mount().map_err(refused)?;
        if matches!(request, Request::Start) {
            {
                let mut all = self
                    .recordings
                    .0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                all.retain(|key, active| {
                    *self.sessions.cancel_signal(key).borrow() == active.epoch
                });
                if all.contains_key(page) {
                    return Err(refused(
                        "An action GIF is already recording for this tab; stop it before starting another.",
                    ));
                }
                if all.len() >= 4 {
                    return Err(refused(
                        "Four action GIFs are already active; stop one before starting another.",
                    ));
                }
                let now = Instant::now();
                all.insert(
                    page.into(),
                    Active {
                        epoch: *self.sessions.cancel_signal(page).borrow(),
                        id: crate::now_ms(),
                        started: now,
                        last: now,
                        frames: Vec::new(),
                        bytes: 0,
                        omitted: 0,
                        reason: None,
                    },
                );
            }
            let status = self.record_frame(page, by).await;
            if status["frames"].as_u64().unwrap_or(0) == 0 {
                self.recordings
                    .0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(page);
                return Err(failed(format!(
                    "Action GIF could not capture its first frame: {}. Make the browser tab visible and retry.",
                    status["reason"]
                )));
            }
            return Ok(
                json!({"ok":true,"recording":status,"Outcome":"Action GIF started with a captured frame. It records subsequent browser actions, without audio."}),
            );
        }
        if !self
            .recordings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(page)
        {
            return Err(refused(
                "No action GIF is recording in this tab; start one before stopping it.",
            ));
        }
        let _ = self.record_frame(page, by).await;
        let active = self
            .recordings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(page)
            .ok_or_else(|| refused("The recording was already stopped; use its returned file."))?;
        bound.check_read_mount().map_err(refused)?;
        let count = active.frames.len();
        let omitted = active.omitted;
        let reason = active.reason;
        let bytes =
            tokio::task::spawn_blocking(move || super::recording_gif::encode(active.frames))
                .await
                .map_err(|_| failed("GIF encoding did not finish; start a shorter recording."))?
                .map_err(failed)?;
        if by == Actor::Agent && self.sessions.stopped(page) {
            return Err(refused(crate::STOPPED));
        }
        Ok(
            json!({"ok":true,"recording":{"state":"stopped","format":"gif","frames":count,"omitted":omitted,"reason":reason,"bytes":bytes.len(),"audio":false},"_vwRecordingGif":base64::engine::general_purpose::STANDARD.encode(bytes),"Outcome":"Action GIF stopped. The core saves the finished GIF in the chat folder."}),
        )
    }

    pub(crate) async fn record_frame(&self, page: &str, by: Actor) -> Value {
        let id = {
            let mut all = self
                .recordings
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let Some(active) = all.get_mut(page) else {
                return Value::Null;
            };
            if by == Actor::Agent
                && (self.sessions.stopped(page)
                    || *self.sessions.cancel_signal(page).borrow() != active.epoch)
            {
                all.remove(page);
                return discarded();
            }
            if active.frames.len() >= MAX_FRAMES
                || active.bytes >= MAX_BYTES
                || active.started.elapsed() >= MAX_TIME
            {
                active.omitted += 1;
                active.reason = Some("The 30-frame, 8 MiB or two-minute recording limit was reached; stop and save this GIF before starting another.".into());
                return status(active);
            }
            active.id
        };
        let captured = tokio::time::timeout(Duration::from_secs(3), async {
            let bound = self.bind_read(page, by).await?;
            let shot = self.service.screenshot(page).await.map_err(|e| crate::driver::preview_text(&e))?;
            bound.check_read_mount()?;
            Ok::<_, String>(shot)
        }).await.unwrap_or_else(|_| Err("The screenshot did not finish within three seconds; make the tab visible before the next action.".into()));
        let mut all = self
            .recordings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if by == Actor::Agent && self.sessions.stopped(page) {
            all.remove(page);
            return discarded();
        }
        let Some(active) = all.get_mut(page).filter(|a| a.id == id) else {
            return Value::Null;
        };
        match captured {
            Ok(shot) if active.bytes + shot.png.len() <= MAX_BYTES => {
                let delay = u16::try_from(active.last.elapsed().as_millis().clamp(100, 5000) / 10)
                    .unwrap_or(50);
                active.bytes += shot.png.len();
                active.frames.push(Frame {
                    png: shot.png,
                    delay,
                });
                active.last = Instant::now();
            }
            Ok(_) => {
                active.omitted += 1;
                active.reason =
                    Some("The recording byte limit was reached; stop and save this GIF.".into());
            }
            Err(reason) => {
                active.omitted += 1;
                active.reason = Some(reason);
            }
        }
        status(active)
    }
}
fn discarded() -> Value {
    json!({"state":"discarded","reason":"The user stopped the browser, so the recording was discarded. Start a new recording in a later turn if the user wants to continue."})
}
fn status(active: &Active) -> Value {
    json!({"state":"recording","format":"gif","frames":active.frames.len(),"omitted":active.omitted,"reason":active.reason,"maxFrames":MAX_FRAMES,"maxSeconds":120,"audio":false})
}
