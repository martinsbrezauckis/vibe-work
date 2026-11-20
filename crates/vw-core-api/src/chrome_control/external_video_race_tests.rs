//! PREDICT: a failed frame or a delayed Finish can leave a helper recording live or admit a new Start too early.
use super::external_video_adversarial_tests::{
    core, request, screenshot, start_result, status, stopped_result,
};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use vw_browser::recording::{VideoBridge, VideoCommand, VideoFuture, VideoReply};

#[derive(Default)]
struct FaultEncoder {
    commands: Mutex<Vec<VideoCommand>>,
    frames: AtomicUsize,
    finish_entered: Arc<Notify>,
    finish_release: Arc<Notify>,
    block_finish: bool,
    fail_second_frame: bool,
}
impl VideoBridge for FaultEncoder {
    fn send(&self, command: VideoCommand) -> VideoFuture<'_> {
        self.commands.lock().unwrap().push(command.clone());
        Box::pin(async move {
            match command {
                VideoCommand::Start { .. } => Ok(VideoReply::Ready),
                VideoCommand::Frame { .. } => {
                    let count = self.frames.fetch_add(1, Ordering::SeqCst) + 1;
                    if self.fail_second_frame && count == 2 {
                        Err("renderer lost".into())
                    } else {
                        Ok(VideoReply::Added)
                    }
                }
                VideoCommand::Finish { .. } => {
                    if self.block_finish {
                        self.finish_entered.notify_one();
                        self.finish_release.notified().await;
                    }
                    Ok(VideoReply::Finished {
                        bytes: b"\x1a\x45\xdf\xa3mock".to_vec(),
                        frames: 1,
                        duration_ms: 300,
                    })
                }
                VideoCommand::Cancel { .. } => Ok(VideoReply::Added),
            }
        })
    }
    fn capture_allowed(&self, _: &str) -> bool {
        true
    }
}

fn mock_sequence(
    core: &crate::Core,
    starts: Arc<AtomicUsize>,
) -> Arc<Mutex<Vec<(String, serde_json::Value)>>> {
    super::test_support::answering(&core.shared, move |tool, args| {
        let token = if starts.load(Ordering::SeqCst) > 1 {
            "video-new"
        } else {
            "video-test"
        };
        match tool {
            "chrome_status" => status(),
            "chrome_execute" if args["action"] == "recording.video.start" => {
                let number = starts.fetch_add(1, Ordering::SeqCst) + 1;
                let mut result = start_result(2);
                result["structuredContent"]["data"]["recordingId"] = json!(if number > 1 {
                    "video-new"
                } else {
                    "video-test"
                });
                result
            }
            "chrome_execute" if args["action"] == "capture.screenshot" => {
                let mut result = screenshot(&args["_approvedCapabilities"]);
                result["structuredContent"]["data"]["captureIdentity"]["recordingId"] =
                    json!(token);
                result
            }
            "chrome_execute" if args["action"] == "recording.video.stop" => {
                let mut result = stopped_result();
                result["structuredContent"]["data"]["recordingId"] = json!(token);
                result["structuredContent"]["data"]["state"] =
                    json!(if args["args"]["discard"] == true {
                        "discarded"
                    } else {
                        "stopped"
                    });
                result
            }
            "chrome_session" => json!({"structuredContent":{"status":"completed"}}),
            _ => panic!("unexpected {tool}: {args}"),
        }
    })
}

#[tokio::test]
async fn lost_encoder_discards_helper_before_new_start() {
    let (core, _) = core("external-video-lost-encoder").await;
    let encoder = Arc::new(FaultEncoder {
        fail_second_frame: true,
        ..FaultEncoder::default()
    });
    core.set_browser_video_bridge(encoder.clone());
    let starts = Arc::new(AtomicUsize::new(0));
    let seen = mock_sequence(&core, starts.clone());
    super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if seen.lock().unwrap().iter().any(|(tool, args)| {
                tool == "chrome_execute" && args["action"] == "recording.video.stop"
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let result = super::read_consent::send(
                &core.shared,
                "s",
                "chrome_execute",
                request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
                None,
            )
            .await;
            if result.is_ok() {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        second.unwrap().value()["structuredContent"]["data"]["recordingId"],
        "video-new"
    );
    assert!(encoder.commands.lock().unwrap().iter().any(
        |command| matches!(command, VideoCommand::Cancel { recording } if recording == "video-test")
    ));
    super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.stop", json!({"tabId":7,"discard":true})),
        None,
    )
    .await
    .unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn stop_holds_session_until_encoder_finish_completes() {
    let (core, _) = core("external-video-finish-reservation").await;
    let encoder = Arc::new(FaultEncoder {
        block_finish: true,
        ..FaultEncoder::default()
    });
    core.set_browser_video_bridge(encoder.clone());
    let starts = Arc::new(AtomicUsize::new(0));
    let _seen = mock_sequence(&core, starts.clone());
    super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await
    .unwrap();
    let shared = core.shared.clone();
    let stop = tokio::spawn(async move {
        super::read_consent::send(
            &shared,
            "s",
            "chrome_execute",
            request(
                "recording.video.stop",
                json!({"tabId":7,"filename":"finish-race.webm"}),
            ),
            None,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), encoder.finish_entered.notified())
        .await
        .unwrap();
    let overlapping = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await;
    assert!(
        overlapping.is_err(),
        "new Start entered while old Finish waited"
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "helper received overlapping Start"
    );
    encoder.finish_release.notify_one();
    let stopped = tokio::time::timeout(Duration::from_secs(3), stop)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        stopped.value()["structuredContent"]["data"]["state"],
        "saved"
    );
    let resumed = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resumed.value()["structuredContent"]["data"]["recordingId"],
        "video-new"
    );
    super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.stop", json!({"tabId":7,"discard":true})),
        None,
    )
    .await
    .unwrap();
    core.shutdown().await;
}

// PREDICT: malformed optional tabId or unknown keys could call helper Start or consume a valid Stop.
#[tokio::test]
async fn malformed_args_never_start_helper_or_consume_active_stop() {
    let (core, _) = core("external-video-malformed-args").await;
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => status(),
        "chrome_execute" if args["action"] == "recording.video.start" => start_result(2),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            screenshot(&args["_approvedCapabilities"])
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => stopped_result(),
        "chrome_session" => json!({"structuredContent":{"status":"completed"}}),
        _ => panic!("unexpected {tool}: {args}"),
    });
    for args in [
        json!({"tabId":null}),
        json!({"tabId":-1}),
        json!({"tabId":"7"}),
        json!({"tabId":2_147_483_648_u64}),
        json!({"extra":1}),
    ] {
        let result = super::read_consent::send(
            &core.shared,
            "s",
            "chrome_execute",
            request("recording.video.start", args),
            None,
        )
        .await;
        assert!(result.is_err(), "malformed Start reached helper");
    }
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute")
    );
    super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await
    .unwrap();
    for args in [
        json!({"tabId":null,"discard":true}),
        json!({"tabId":-1,"discard":true}),
        json!({"tabId":"7","discard":true}),
        json!({"tabId":2_147_483_648_u64,"discard":true}),
        json!({"extra":1,"discard":true}),
    ] {
        let result = super::read_consent::send(
            &core.shared,
            "s",
            "chrome_execute",
            request("recording.video.stop", args),
            None,
        )
        .await;
        assert!(result.is_err(), "malformed Stop consumed recording");
    }
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(
                |(tool, args)| tool == "chrome_execute" && args["action"] == "recording.video.stop"
            )
            .count(),
        0
    );
    let stopped = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.stop", json!({"tabId":7,"discard":true})),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        stopped.value()["structuredContent"]["data"]["state"],
        "discarded"
    );
    core.shutdown().await;
}

// PREDICT: a foreground browser action currently leaves the frame operation lock free while its helper reply is pending.
