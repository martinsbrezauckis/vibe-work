//! Bounded page video from guarded native screenshots into the app's trusted `WebCodecs` encoder.
//! The page never supplies frames or runs the encoder. A Stop cancels the worker before a file can be saved.
use crate::driver::{failed, refused};
use crate::recording::Request;
use crate::{ActError, Actor, Browser};
use base64::Engine as _;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinHandle;

const MAX_VIDEO_TIME: Duration = Duration::from_secs(300);
const MAX_VIDEO_FRAMES: usize = 1200;
const MAX_PNG_BYTES: usize = 2 * 1024 * 1024;
const FRAME_INTERVAL: Duration = Duration::from_millis(250);
static NEXT_VIDEO_ID: AtomicU64 = AtomicU64::new(1);

/// The app's bounded answer to one encoder command.
#[derive(Clone, Debug)]
pub enum VideoReply {
    /// VP8 is available and a new encoder was opened.
    Ready,
    /// VP8 is unavailable in the trusted app webview.
    Unsupported,
    /// A guarded frame was accepted, or an encoder was cancelled.
    Added,
    /// Complete `WebM` bytes and the encoder's actual frame count and duration.
    Finished {
        /// Encoded `WebM` bytes.
        bytes: Vec<u8>,
        /// Number of encoded frames.
        frames: usize,
        /// Encoded duration in milliseconds.
        duration_ms: u64,
    },
}

/// Requests carry only frames already admitted by the native browser's capture guard.
#[derive(Clone, Debug)]
pub enum VideoCommand {
    /// Open one encoder after the codec check.
    Start {
        /// Opaque recording identity.
        recording: String,
    },
    /// Encode one guarded native screenshot.
    Frame {
        /// Opaque recording identity.
        recording: String,
        /// PNG bytes from the guarded native screenshot.
        png: Vec<u8>,
        /// Time since recording start, in seconds.
        at_seconds: f64,
    },
    /// Flush the encoder and return a `WebM` file.
    Finish {
        /// Opaque recording identity.
        recording: String,
        /// Time since recording start, in seconds.
        at_seconds: f64,
    },
    /// Discard the encoder without a file.
    Cancel {
        /// Opaque recording identity.
        recording: String,
    },
}

/// One encoder reply, bounded by the caller's timeout.
pub type VideoFuture<'a> = Pin<Box<dyn Future<Output = Result<VideoReply, String>> + Send + 'a>>;

/// A host-owned encoder. `capture_allowed` is false when the person has paused or stopped this browser.
pub trait VideoBridge: Send + Sync {
    /// Sends one request to the trusted encoder and waits for its reply.
    fn send(&self, request: VideoCommand) -> VideoFuture<'_>;
    /// Whether the person has left this page's browser under agent control.
    fn capture_allowed(&self, page: &str) -> bool;
}

struct ActiveVideo {
    recording: String,
    epoch: u64,
    started: Instant,
    full_seen: Arc<AtomicBool>,
    cancel: watch::Sender<bool>,
    worker: JoinHandle<Stats>,
}

struct StartReservation(Arc<Mutex<HashSet<String>>>, String);
impl Drop for StartReservation {
    fn drop(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.1);
    }
}

#[derive(Default)]
struct Stats {
    frames: usize,
    omitted: usize,
    reason: Option<String>,
}

struct CaptureContext<'a> {
    epoch: u64,
    bridge: &'a Arc<dyn VideoBridge>,
    recording: &'a str,
    started: Instant,
    full_seen: &'a AtomicBool,
}

/// Per-browser recording state; cloned browser handles see the same sessions.
#[derive(Clone, Default)]
pub struct VideoRecordings {
    bridge: Arc<RwLock<Option<Arc<dyn VideoBridge>>>>,
    active: Arc<Mutex<HashMap<String, ActiveVideo>>>,
    starting: Arc<Mutex<HashSet<String>>>,
}
impl VideoRecordings {
    pub(crate) fn has_active(&self, page: &str) -> bool {
        if self
            .starting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(page)
        {
            return true;
        }
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(page)
    }
}

impl Browser {
    /// Installs the app's encoder route after the host starts. Headless runs keep the explicit GIF alternative.
    pub fn set_video_bridge(&self, bridge: Arc<dyn VideoBridge>) {
        *self
            .video
            .bridge
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(bridge);
    }

    pub(crate) async fn record_video(
        &self,
        page: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bridge = self
            .video
            .bridge
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| refused(unavailable()))?;
        let bound = self.bind_read(page, by).await.map_err(refused)?;
        bound.check_read_mount().map_err(refused)?;
        if matches!(request, Request::VideoStart) {
            return self.start_video(page, by, bridge).await;
        }
        let active = self
            .video
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(page)
            .ok_or_else(|| refused(copy("browserVideoNoActive")))?;
        active.cancel.send_replace(true);
        let full_seen = active.full_seen.clone();
        let mut worker = active.worker;
        let Ok(Ok(stats)) = tokio::time::timeout(Duration::from_secs(12), &mut worker).await else {
            worker.abort();
            let _ = worker.await;
            cancel_video(&bridge, &active.recording).await;
            return Err(failed(copy("browserVideoStopTimeout")));
        };
        if (by == Actor::Agent
            && (self.sessions.stopped(page)
                || *self.sessions.cancel_signal(page).borrow() != active.epoch))
            || !bridge.capture_allowed(page)
            || (full_seen.load(Ordering::SeqCst) && !self.full_capture_authorized(page))
        {
            cancel_video(&bridge, &active.recording).await;
            return Err(refused(crate::STOPPED));
        }
        if let Err(reason) = bound.check_read_mount() {
            cancel_video(&bridge, &active.recording).await;
            return Err(refused(reason));
        }
        let finished = tokio::time::timeout(
            Duration::from_secs(15),
            bridge.send(VideoCommand::Finish {
                recording: active.recording.clone(),
                at_seconds: active.started.elapsed().min(MAX_VIDEO_TIME).as_secs_f64(),
            }),
        )
        .await;
        let answer = match finished {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => {
                cancel_video(&bridge, &active.recording).await;
                return Err(failed(copy("browserVideoEncodeFailed")));
            }
            Err(_) => {
                cancel_video(&bridge, &active.recording).await;
                return Err(failed(copy("browserVideoEncodeTimeout")));
            }
        };
        let VideoReply::Finished {
            bytes,
            frames,
            duration_ms,
        } = answer
        else {
            return Err(failed(copy("browserVideoEncodeFailed")));
        };
        if bytes.len() > 32 * 1024 * 1024 || !bytes.starts_with(b"\x1a\x45\xdf\xa3") {
            return Err(failed(copy("browserVideoInvalidBytes")));
        }
        if (by == Actor::Agent
            && (self.sessions.stopped(page)
                || *self.sessions.cancel_signal(page).borrow() != active.epoch))
            || !bridge.capture_allowed(page)
            || (full_seen.load(Ordering::SeqCst) && !self.full_capture_authorized(page))
            || self
                .dispatch_permission_check(page, by)
                .is_some_and(|check| !check())
        {
            return Err(refused(crate::STOPPED));
        }
        bound.check_read_mount().map_err(refused)?;
        Ok(
            json!({"ok":true,"recording":{"state":"stopped","format":"webm","frames":frames,"omitted":stats.omitted,"reason":stats.reason,"durationMs":duration_ms,"bytes":bytes.len(),"audio":false},"_vwRecordingWebm":base64::engine::general_purpose::STANDARD.encode(bytes),"Outcome":copy("browserVideoStopped")}),
        )
    }

    // Keep reservation, first guarded frame and spawned cancellation worker in one lifecycle boundary.
    #[allow(clippy::too_many_lines)]
    async fn start_video(
        &self,
        page: &str,
        by: Actor,
        bridge: Arc<dyn VideoBridge>,
    ) -> Result<Value, ActError> {
        if self.recordings.has_active(page) {
            return Err(refused(copy("browserVideoGifActive")));
        }
        let stale = {
            let mut starting = self
                .video
                .starting
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let mut active = self
                .video
                .active
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if starting.contains(page) {
                return Err(refused(copy("browserVideoStarting")));
            }
            let stale = active.get(page).is_some_and(|entry| {
                entry.worker.is_finished()
                    || *self.sessions.cancel_signal(page).borrow() != entry.epoch
            });
            if !stale && active.contains_key(page) {
                return Err(refused(copy("browserVideoActive")));
            }
            if active.len() >= 4 && !stale {
                return Err(refused(copy("browserVideoFourActive")));
            }
            starting.insert(page.to_owned());
            if stale { active.remove(page) } else { None }
        };
        let _reservation = StartReservation(self.video.starting.clone(), page.to_owned());
        if let Some(old) = stale {
            old.cancel.send_replace(true);
            old.worker.abort();
            cancel_video(&bridge, &old.recording).await;
        }
        let epoch = *self.sessions.cancel_signal(page).borrow();
        let recording = format!("video-{}", NEXT_VIDEO_ID.fetch_add(1, Ordering::Relaxed));
        let start = tokio::time::timeout(
            Duration::from_secs(5),
            bridge.send(VideoCommand::Start {
                recording: recording.clone(),
            }),
        )
        .await;
        match start {
            Ok(Ok(VideoReply::Ready)) => {}
            Ok(Ok(VideoReply::Unsupported)) => return Err(refused(unavailable())),
            _ => {
                cancel_video(&bridge, &recording).await;
                return Err(failed(copy("browserVideoStartFailed")));
            }
        }
        let started = Instant::now();
        let full_seen = Arc::new(AtomicBool::new(false));
        match self
            .capture_video_frame(
                page,
                by,
                CaptureContext {
                    epoch,
                    bridge: &bridge,
                    recording: &recording,
                    started,
                    full_seen: &full_seen,
                },
            )
            .await
        {
            Ok(()) => {}
            Err(error) => {
                cancel_video(&bridge, &recording).await;
                if let Some(refusal) = crate::protected_capture::first_frame_refusal(&error) {
                    return Err(refused(refusal));
                }
                return Err(failed(
                    copy("browserVideoFirstFrameFailed").replace("{reason}", &error),
                ));
            }
        }
        let (cancel, mut cancelled) = watch::channel(false);
        let browser = self.clone();
        let page_owned = page.to_owned();
        let recording_owned = recording.clone();
        let worker_bridge = bridge.clone();
        let worker_full_seen = full_seen.clone();
        let worker = tokio::spawn(async move {
            let mut stats = Stats {
                frames: 1,
                ..Stats::default()
            };
            let mut interval = tokio::time::interval(FRAME_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await; // first frame was captured synchronously
            let mut epoch_changed = browser.sessions.cancel_signal(&page_owned);
            let mut person_stopped = false;
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = cancelled.changed() => break,
                    _ = epoch_changed.changed() => { person_stopped = true; break; },
                }
                if *cancelled.borrow() {
                    break;
                }
                if browser.sessions.stopped(&page_owned) || *epoch_changed.borrow() != epoch {
                    person_stopped = true;
                    break;
                }
                if (by == Actor::Agent
                    && (!browser.sessions.controlled(&page_owned)
                        || browser
                            .dispatch_permission_check(&page_owned, by)
                            .is_some_and(|check| !check())))
                    || (worker_full_seen.load(Ordering::SeqCst)
                        && !browser.full_capture_authorized(&page_owned))
                {
                    person_stopped = true;
                    break;
                }
                if started.elapsed() >= MAX_VIDEO_TIME || stats.frames >= MAX_VIDEO_FRAMES {
                    stats
                        .reason
                        .get_or_insert_with(|| copy("browserVideoLimit").into());
                    continue;
                }
                if !worker_bridge.capture_allowed(&page_owned) {
                    continue;
                }
                match browser
                    .capture_video_frame(
                        &page_owned,
                        by,
                        CaptureContext {
                            epoch,
                            bridge: &worker_bridge,
                            recording: &recording_owned,
                            started,
                            full_seen: &worker_full_seen,
                        },
                    )
                    .await
                {
                    Ok(()) => stats.frames += 1,
                    Err(error)
                        if error == copy("browserVideoFrameRefused")
                            || error == copy("browserVideoFrameTimeout") =>
                    {
                        stats.omitted += 1;
                        stats.reason = Some(error);
                        person_stopped = true;
                        break;
                    }
                    Err(error) => {
                        stats.omitted += 1;
                        stats.reason = Some(error);
                    }
                }
            }
            if person_stopped {
                cancel_video(&worker_bridge, &recording_owned).await;
                let mut active = browser
                    .video
                    .active
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if active
                    .get(&page_owned)
                    .is_some_and(|entry| entry.recording == recording_owned)
                {
                    active.remove(&page_owned);
                }
            }
            stats
        });
        self.video
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                page.to_owned(),
                ActiveVideo {
                    recording,
                    epoch,
                    started,
                    full_seen,
                    cancel,
                    worker,
                },
            );
        Ok(
            json!({"ok":true,"recording":{"state":"recording","format":"webm","frames":1,"audio":false},"Outcome":copy("browserVideoStarted")}),
        )
    }

    fn full_capture_authorized(&self, page: &str) -> bool {
        self.delegation_authorizer.read().is_ok_and(|slot| {
            slot.as_ref()
                .is_some_and(|authorizer| authorizer.protected_values_delegated(page))
        })
    }

    async fn capture_video_frame(
        &self,
        page: &str,
        by: Actor,
        context: CaptureContext<'_>,
    ) -> Result<(), String> {
        let CaptureContext {
            epoch,
            bridge,
            recording,
            started,
            full_seen,
        } = context;
        if !bridge.capture_allowed(page) {
            return Err(copy("browserVideoPaused").into());
        }
        if by == Actor::Agent && !self.sessions.controlled(page) {
            return Err(copy("browserVideoReleased").into());
        }
        let allowed = self.dispatch_permission_check(page, by);
        if allowed.as_ref().is_some_and(|check| !check()) {
            return Err(copy("browserVideoUnauthorized").into());
        }
        let full_before = self.full_capture_authorized(page);
        if full_before {
            full_seen.store(true, Ordering::SeqCst);
        }
        let shot = tokio::time::timeout(Duration::from_secs(3), async {
            let bound = self.bind_read(page, by).await?;
            self.admit_capture(page).await?;
            let shot = self
                .service
                .screenshot(page)
                .await
                .map_err(|e| crate::driver::preview_text(&e))?;
            bound.check_read_mount()?;
            self.admit_capture(page).await?;
            if allowed.as_ref().is_some_and(|check| !check()) {
                return Err(copy("browserVideoUnauthorized").into());
            }
            Ok::<_, String>(shot)
        })
        .await
        .map_err(|_| copy("browserVideoScreenshotTimeout").to_owned())??;
        if shot.png.len() > MAX_PNG_BYTES {
            return Err(copy("browserVideoFrameTooLarge").into());
        }
        if (by == Actor::Agent
            && (self.sessions.stopped(page)
                || *self.sessions.cancel_signal(page).borrow() != epoch))
            || !bridge.capture_allowed(page)
            || allowed.as_ref().is_some_and(|check| !check())
        {
            return Err(copy("browserVideoCaptureRevoked").into());
        }
        let full = full_before || self.full_capture_authorized(page);
        if full {
            full_seen.store(true, Ordering::SeqCst);
        }
        if full && !self.full_capture_authorized(page) {
            return Err(copy("browserVideoFullEnded").into());
        }
        match tokio::time::timeout(
            Duration::from_secs(8),
            bridge.send(VideoCommand::Frame {
                recording: recording.to_owned(),
                png: shot.png,
                at_seconds: started.elapsed().as_secs_f64(),
            }),
        )
        .await
        .map_err(|_| copy("browserVideoFrameTimeout").to_owned())?
        .map_err(|_| copy("browserVideoFrameRefused").to_owned())?
        {
            VideoReply::Added
                if (by != Actor::Agent
                    || (!self.sessions.stopped(page)
                        && *self.sessions.cancel_signal(page).borrow() == epoch))
                    && bridge.capture_allowed(page)
                    && allowed.as_ref().is_none_or(|check| check())
                    && (!full || self.full_capture_authorized(page)) =>
            {
                Ok(())
            }
            _ => Err(copy("browserVideoFrameRefused").into()),
        }
    }
}

fn unavailable() -> &'static str {
    copy("browserVideoUnavailable")
}

async fn cancel_video(bridge: &Arc<dyn VideoBridge>, recording: &str) {
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        bridge.send(VideoCommand::Cancel {
            recording: recording.to_owned(),
        }),
    )
    .await;
}

fn copy(key: &str) -> &'static str {
    vw_vocabulary::text(key).unwrap_or_default()
}
