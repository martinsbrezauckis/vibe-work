//! Action GIF receipts contain actual captures, reject a second stop and stop collecting at the person's Stop.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used)]
mod support;
use base64::Engine as _;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use support::{Rig, SESSION};
use vw_browser::Actor;
use vw_browser::recording::{VideoBridge, VideoCommand, VideoFuture, VideoReply};
struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}
async fn act(rig: &Rig, args: Value) -> Value {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&args).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn action_gif_contains_captured_frames_and_stop_is_final() {
    let rig = Rig::new().await;
    let started = act(&rig, json!({"action":"recording.start"})).await;
    assert_eq!(started["recording"]["state"], "recording");
    rig.engine
        .eval("document.body.style.background='red'")
        .await;
    act(&rig, json!({"action":"media.read"})).await;
    let stopped = act(&rig, json!({"action":"recording.stop"})).await;
    assert!(stopped["recording"]["frames"].as_u64().unwrap() >= 2);
    assert!(stopped["_vwRecordingGif"].is_string());
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(stopped["_vwRecordingGif"].as_str().unwrap())
        .unwrap();
    let mut decoder = gif::DecodeOptions::new();
    decoder.set_color_output(gif::ColorOutput::RGBA);
    let mut reader = decoder.read_info(bytes.as_slice()).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_next_frame().unwrap() {
        frames.push(frame.buffer.to_vec());
    }
    assert_eq!(
        frames.len() as u64,
        stopped["recording"]["frames"].as_u64().unwrap()
    );
    assert_ne!(
        frames[0], frames[1],
        "GIF must contain the changed page pixels"
    );
    assert!(
        rig.browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"recording.stop"})).unwrap(),
                Actor::Agent,
                &NoNavigation
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stop_discards_the_recording_and_video_offers_the_gif_alternative() {
    let rig = Rig::new().await;
    act(&rig, json!({"action":"recording.start"})).await;
    rig.browser.sessions().stop(SESSION);
    assert!(
        rig.browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"recording.stop"})).unwrap(),
                Actor::Agent,
                &NoNavigation
            )
            .await
            .is_err()
    );
    rig.browser.sessions().resume(SESSION);
    let restarted = act(&rig, json!({"action":"recording.start"})).await;
    assert_eq!(restarted["recording"]["frames"], 1);
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"recording.video.start"})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert!(error.text.contains("recording.start") && error.text.contains("encoder"));
}

// PREDICT: an installed encoder receives guarded PNG frames, a bounded video can stop once,
// and the old action GIF remains the explicit fallback only when no encoder is installed.
#[derive(Default)]
struct TestVideoBridge(Mutex<Vec<VideoCommand>>);
impl VideoBridge for TestVideoBridge {
    fn send(&self, request: VideoCommand) -> VideoFuture<'_> {
        self.0.lock().unwrap().push(request.clone());
        Box::pin(async move {
            Ok(match request {
                VideoCommand::Start { .. } => VideoReply::Ready,
                VideoCommand::Frame { .. } | VideoCommand::Cancel { .. } => VideoReply::Added,
                VideoCommand::Finish { .. } => VideoReply::Finished {
                    bytes: b"\x1a\x45\xdf\xa3mock".to_vec(),
                    frames: 1,
                    duration_ms: 300,
                },
            })
        })
    }
    fn capture_allowed(&self, _: &str) -> bool {
        true
    }
}

#[tokio::test]
async fn video_uses_guarded_frames_and_stop_is_final() {
    let rig = Rig::new().await;
    let bridge = Arc::new(TestVideoBridge::default());
    rig.browser.set_video_bridge(bridge.clone());
    let started = act(&rig, json!({"action":"recording.video.start"})).await;
    assert_eq!(started["recording"]["format"], "webm");
    tokio::time::sleep(std::time::Duration::from_millis(650)).await;
    let stopped = act(&rig, json!({"action":"recording.video.stop"})).await;
    assert_eq!(stopped["recording"]["format"], "webm");
    assert!(stopped["_vwRecordingWebm"].is_string());
    {
        let calls = bridge.0.lock().unwrap();
        assert!(calls.iter().any(
            |call| matches!(call, VideoCommand::Frame { png, .. } if png.starts_with(b"\x89PNG"))
        ));
        assert!(
            calls
                .iter()
                .any(|call| matches!(call, VideoCommand::Finish { .. }))
        );
    }
    assert!(
        rig.browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"recording.video.stop"})).unwrap(),
                Actor::Agent,
                &NoNavigation
            )
            .await
            .is_err()
    );
}

// PREDICT: if video capture bypasses the screenshot guard, a visible protected value can enter an encoder frame.
#[tokio::test]
async fn protected_field_value_never_reaches_a_video_frame() {
    let rig = Rig::new().await;
    let bridge = Arc::new(TestVideoBridge::default());
    rig.browser.set_video_bridge(bridge.clone());
    rig.engine
        .eval("document.body.innerHTML='<label>Card number <input value=4111111111119917></label>'")
        .await;
    let outcome = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"recording.video.start"})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await;
    let error = outcome.expect_err("visible card value must refuse video capture");
    assert!(
        error
            .text
            .contains("Capture was withheld so the value stays private"),
        "{error:?}"
    );
    let calls = bridge.0.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, VideoCommand::Cancel { .. }))
    );
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, VideoCommand::Frame { .. }))
    );
}

// PREDICT: a person's Stop discards the encoder, and Resume may start a fresh video in the same tab.
#[tokio::test]
async fn stopped_video_is_cancelled_and_a_new_turn_can_record() {
    let rig = Rig::new().await;
    let bridge = Arc::new(TestVideoBridge::default());
    rig.browser.set_video_bridge(bridge.clone());
    act(&rig, json!({"action":"recording.video.start"})).await;
    rig.browser.sessions().stop(SESSION);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    rig.browser.sessions().resume(SESSION);
    let restarted = act(&rig, json!({"action":"recording.video.start"})).await;
    assert_eq!(restarted["recording"]["format"], "webm");
    assert!(
        bridge
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, VideoCommand::Cancel { .. }))
    );
}

// PREDICT: Stop+Resume while the encoder flushes invalidates the old epoch; no stale WebM receipt is returned.
struct DelayedFinish {
    started: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
}
impl VideoBridge for DelayedFinish {
    fn send(&self, request: VideoCommand) -> VideoFuture<'_> {
        Box::pin(async move {
            Ok(match request {
                VideoCommand::Start { .. } => VideoReply::Ready,
                VideoCommand::Frame { .. } | VideoCommand::Cancel { .. } => VideoReply::Added,
                VideoCommand::Finish { .. } => {
                    self.started.notify_one();
                    self.resume.notified().await;
                    VideoReply::Finished {
                        bytes: b"\x1a\x45\xdf\xa3mock".to_vec(),
                        frames: 1,
                        duration_ms: 300,
                    }
                }
            })
        })
    }
    fn capture_allowed(&self, _: &str) -> bool {
        true
    }
}

#[tokio::test]
async fn stop_resume_during_finish_discards_stale_webm() {
    let rig = Rig::new().await;
    let bridge = Arc::new(DelayedFinish {
        started: Arc::new(tokio::sync::Notify::new()),
        resume: Arc::new(tokio::sync::Notify::new()),
    });
    rig.browser.set_video_bridge(bridge.clone());
    act(&rig, json!({"action":"recording.video.start"})).await;
    let browser = rig.browser.clone();
    let stopping = tokio::spawn(async move {
        browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"recording.video.stop"})).unwrap(),
                Actor::Agent,
                &NoNavigation,
            )
            .await
    });
    bridge.started.notified().await;
    rig.browser.sessions().stop(SESSION);
    rig.browser.sessions().resume(SESSION);
    bridge.resume.notify_one();
    assert!(stopping.await.unwrap().is_err());
}

// PREDICT: two chats may record independently, while two starts in one tab reserve only one encoder.
struct SlowStart;
impl VideoBridge for SlowStart {
    fn send(&self, request: VideoCommand) -> VideoFuture<'_> {
        Box::pin(async move {
            Ok(match request {
                VideoCommand::Start { .. } => {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    VideoReply::Ready
                }
                VideoCommand::Frame { .. } | VideoCommand::Cancel { .. } => VideoReply::Added,
                VideoCommand::Finish { .. } => VideoReply::Finished {
                    bytes: b"\x1a\x45\xdf\xa3mock".to_vec(),
                    frames: 1,
                    duration_ms: 300,
                },
            })
        })
    }
    fn capture_allowed(&self, _: &str) -> bool {
        true
    }
}

#[tokio::test]
async fn parallel_video_starts_reserve_one_tab_once() {
    let rig = Rig::new().await;
    rig.browser.set_video_bridge(Arc::new(SlowStart));
    let a = rig.browser.act(
        SESSION,
        vw_browser::parse(&json!({"action":"recording.video.start"})).unwrap(),
        Actor::Agent,
        &NoNavigation,
    );
    let b = rig.browser.act(
        SESSION,
        vw_browser::parse(&json!({"action":"recording.video.start"})).unwrap(),
        Actor::Agent,
        &NoNavigation,
    );
    let (a, b) = tokio::join!(a, b);
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
}

struct PausableBridge {
    inner: TestVideoBridge,
    allowed: std::sync::atomic::AtomicBool,
}
impl VideoBridge for PausableBridge {
    fn send(&self, request: VideoCommand) -> VideoFuture<'_> {
        self.inner.send(request)
    }
    fn capture_allowed(&self, _: &str) -> bool {
        self.allowed.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[tokio::test]
async fn pause_holds_frames_resume_continues_and_turn_end_discards() {
    let rig = Rig::new().await;
    let bridge = Arc::new(PausableBridge {
        inner: TestVideoBridge::default(),
        allowed: std::sync::atomic::AtomicBool::new(true),
    });
    rig.browser.set_video_bridge(bridge.clone());
    act(&rig, json!({"action":"recording.video.start"})).await;
    bridge
        .allowed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let frames = || {
        bridge
            .inner
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, VideoCommand::Frame { .. }))
            .count()
    };
    let before = frames();
    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    assert_eq!(frames(), before, "Pause must hold frames");
    bridge
        .allowed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    assert!(frames() > before, "Resume must continue frames");
    rig.browser.sessions().cancel_confirmations(SESSION);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        bridge
            .inner
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, VideoCommand::Cancel { .. })),
        "Turn end must discard its encoder"
    );
}

struct SwitchingAccess(std::sync::atomic::AtomicBool);
impl vw_browser::delegation::DelegationAuthorizer for SwitchingAccess {
    fn protected_values_delegated(&self, _: &str) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[tokio::test]
async fn full_access_downgrade_discards_prior_video_frames() {
    let rig = Rig::new().await;
    let bridge = Arc::new(TestVideoBridge::default());
    let access = Arc::new(SwitchingAccess(std::sync::atomic::AtomicBool::new(true)));
    rig.browser.set_video_bridge(bridge.clone());
    rig.browser.set_delegation_authorizer(access.clone());
    act(&rig, json!({"action":"recording.video.start"})).await;
    access.0.store(false, std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        bridge
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, VideoCommand::Cancel { .. }))
    );
    assert!(
        rig.browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"recording.video.stop"})).unwrap(),
                Actor::Agent,
                &NoNavigation
            )
            .await
            .is_err()
    );
}

// PREDICT: optional recording formats must match the action; missing format keeps each action's default.
#[test]
fn recording_format_is_specific_to_gif_or_webm() {
    for action in ["recording.video.start", "recording.video.stop"] {
        assert!(vw_browser::parse(&json!({"action":action})).is_ok());
        assert!(vw_browser::parse(&json!({"action":action,"format":"webm"})).is_ok());
        assert!(vw_browser::parse(&json!({"action":action,"format":"gif"})).is_err());
        assert!(vw_browser::parse(&json!({"action":action,"format":""})).is_err());
    }
    for action in ["recording.start", "recording.stop"] {
        assert!(vw_browser::parse(&json!({"action":action})).is_ok());
        assert!(vw_browser::parse(&json!({"action":action,"format":"gif"})).is_ok());
        assert!(vw_browser::parse(&json!({"action":action,"format":"webm"})).is_err());
    }
}

#[test]
fn page_video_copy_keys_have_nonempty_vocabulary_entries() {
    let vocabulary: Value =
        serde_json::from_str(include_str!("../../vw-vocabulary/vocabulary.json")).unwrap();
    for source in [
        include_str!("../src/recording_video.rs"),
        include_str!("../src/act.rs"),
        include_str!("../src/recording.rs"),
        include_str!("../../vw-core-api/src/browser_recording.rs"),
        include_str!("../../vw-agent-tools/src/browser_everyday.rs"),
    ] {
        for part in source
            .split('"')
            .filter(|part| part.starts_with("browserVideo") || *part == "browserGifFormat")
        {
            assert!(
                vocabulary["texts"][part]
                    .as_str()
                    .is_some_and(|text| !text.is_empty()),
                "missing vocabulary key {part}"
            );
        }
    }
}

// PREDICT: a start request that times out is followed by Cancel, so a late UI start cannot orphan an encoder.
#[derive(Default)]
struct NeverStart(Mutex<Vec<VideoCommand>>);
impl VideoBridge for NeverStart {
    fn send(&self, request: VideoCommand) -> VideoFuture<'_> {
        self.0.lock().unwrap().push(request.clone());
        Box::pin(async move {
            match request {
                VideoCommand::Start { .. } => std::future::pending().await,
                _ => Ok(VideoReply::Added),
            }
        })
    }
    fn capture_allowed(&self, _: &str) -> bool {
        true
    }
}

#[tokio::test]
async fn timed_out_start_cancels_late_encoder() {
    let rig = Rig::new().await;
    let bridge = Arc::new(NeverStart::default());
    rig.browser.set_video_bridge(bridge.clone());
    assert!(
        rig.browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"recording.video.start"})).unwrap(),
                Actor::Agent,
                &NoNavigation
            )
            .await
            .is_err()
    );
    assert!(
        bridge
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, VideoCommand::Cancel { .. }))
    );
}

struct DelayedSecondFrame {
    calls: Mutex<Vec<VideoCommand>>,
    frames: std::sync::atomic::AtomicUsize,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl VideoBridge for DelayedSecondFrame {
    fn send(&self, request: VideoCommand) -> VideoFuture<'_> {
        self.calls.lock().unwrap().push(request.clone());
        Box::pin(async move {
            Ok(match request {
                VideoCommand::Start { .. } => VideoReply::Ready,
                VideoCommand::Frame { .. } => {
                    if self
                        .frames
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        == 1
                    {
                        self.entered.notify_one();
                        self.resume.notified().await;
                    }
                    VideoReply::Added
                }
                VideoCommand::Cancel { .. } => VideoReply::Added,
                VideoCommand::Finish { .. } => VideoReply::Finished {
                    bytes: b"\x1a\x45\xdf\xa3mock".to_vec(),
                    frames: 2,
                    duration_ms: 500,
                },
            })
        })
    }
    fn capture_allowed(&self, _: &str) -> bool {
        true
    }
}

// PREDICT: a Full-only frame in flight at downgrade poisons the entire video even when its ACK arrives after downgrade.
#[tokio::test]
async fn in_flight_full_frame_downgrade_discards_video() {
    let rig = Rig::new().await;
    let bridge = Arc::new(DelayedSecondFrame {
        calls: Mutex::new(Vec::new()),
        frames: std::sync::atomic::AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    let access = Arc::new(SwitchingAccess(std::sync::atomic::AtomicBool::new(false)));
    rig.browser.set_video_bridge(bridge.clone());
    rig.browser.set_delegation_authorizer(access.clone());
    act(&rig, json!({"action":"recording.video.start"})).await;
    access.0.store(true, std::sync::atomic::Ordering::SeqCst);
    bridge.entered.notified().await;
    access.0.store(false, std::sync::atomic::Ordering::SeqCst);
    bridge.resume.notify_one();
    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    assert!(
        bridge
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, VideoCommand::Cancel { .. }))
    );
}

struct FailedSecondFrame {
    calls: Mutex<Vec<VideoCommand>>,
    frames: std::sync::atomic::AtomicUsize,
}
impl VideoBridge for FailedSecondFrame {
    fn send(&self, request: VideoCommand) -> VideoFuture<'_> {
        self.calls.lock().unwrap().push(request.clone());
        Box::pin(async move {
            match request {
                VideoCommand::Start { .. } => Ok(VideoReply::Ready),
                VideoCommand::Frame { .. }
                    if self
                        .frames
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        > 0 =>
                {
                    Err("renderer lost".into())
                }
                VideoCommand::Frame { .. } | VideoCommand::Cancel { .. } => Ok(VideoReply::Added),
                VideoCommand::Finish { .. } => Ok(VideoReply::Finished {
                    bytes: b"\x1a\x45\xdf\xa3mock".to_vec(),
                    frames: 1,
                    duration_ms: 300,
                }),
            }
        })
    }
    fn capture_allowed(&self, _: &str) -> bool {
        true
    }
}

// PREDICT: a lost encoder after the first frame stops native capture and discards its session.
#[tokio::test]
async fn encoder_failure_discards_video_instead_of_continuing_capture() {
    let rig = Rig::new().await;
    let bridge = Arc::new(FailedSecondFrame {
        calls: Mutex::new(Vec::new()),
        frames: std::sync::atomic::AtomicUsize::new(0),
    });
    rig.browser.set_video_bridge(bridge.clone());
    act(&rig, json!({"action":"recording.video.start"})).await;
    tokio::time::sleep(std::time::Duration::from_millis(650)).await;
    let calls = bridge.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, VideoCommand::Cancel { .. }))
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, VideoCommand::Frame { .. }))
            .count(),
        2
    );
}
