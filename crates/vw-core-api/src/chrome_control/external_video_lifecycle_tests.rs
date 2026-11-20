//! Lifecycle boundary tests for the guarded external video worker.
//! PREDICT: Pause and duration limits may leave stale encoders or wrong save receipts.
use super::external_video_adversarial_tests::{
    core, request, saved_stop_result, screenshot, start_result, status, stopped_result,
};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use vw_browser::recording::VideoCommand;

// PREDICT: a duration-limited worker could discard its safe frames or omit the cap receipt on Stop.
#[tokio::test]
async fn duration_cap_saves_bounded_webm_and_receipt() {
    let (core, encoder) = core("external-video-duration-cap").await;
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => status(),
        "chrome_execute" if args["action"] == "recording.video.start" => start_result(1),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            screenshot(&args["_approvedCapabilities"])
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => saved_stop_result(),
        _ => panic!("unexpected {tool}: {args}"),
    });
    super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":1})),
        None,
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let result = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request(
            "recording.video.stop",
            json!({"tabId":7,"filename":"duration-cap.webm"}),
        ),
        None,
    )
    .await
    .unwrap();
    let data = &result.value()["structuredContent"]["data"];
    assert_eq!(data["state"], "saved");
    assert_eq!(data["limit"], "duration");
    assert!(data["frames"].as_u64().unwrap() >= 1);
    assert!(data["frames"].as_u64().unwrap() <= 1200);
    assert!(data["durationMs"].as_u64().unwrap() <= 1000);
    assert!(data["bytes"].as_u64().unwrap() <= 32 * 1024 * 1024);
    assert_eq!(data["file"]["path"], "duration-cap.webm");
    assert!(
        encoder
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, VideoCommand::Finish { .. }))
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(
                |(tool, args)| tool == "chrome_execute" && args["action"] == "recording.video.stop"
            )
            .count(),
        1
    );
    core.shutdown().await;
}

fn pause_answer(
    tool: &str,
    args: &serde_json::Value,
    paused: &std::sync::atomic::AtomicBool,
    starts: &std::sync::atomic::AtomicUsize,
) -> serde_json::Value {
    let token = if starts.load(Ordering::SeqCst) >= 2 {
        "video-resume"
    } else {
        "video-test"
    };
    match tool {
        "chrome_status" => {
            let mut data = status();
            if paused.load(Ordering::SeqCst) {
                data["structuredContent"]["sessions"][0]["state"] = json!("paused");
            }
            data
        }
        "chrome_execute" if args["action"] == "recording.video.start" => {
            let ordinal = starts.fetch_add(1, Ordering::SeqCst);
            let mut data = start_result(2);
            if ordinal > 0 {
                data["structuredContent"]["data"]["recordingId"] = json!("video-resume");
            }
            data
        }
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            let mut data = screenshot(&args["_approvedCapabilities"]);
            data["structuredContent"]["data"]["captureIdentity"]["recordingId"] = json!(token);
            data
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => {
            let mut data = stopped_result();
            data["structuredContent"]["data"]["recordingId"] = json!(token);
            data
        }
        _ => panic!("unexpected {tool}: {args}"),
    }
}

// PREDICT: Pause could leave an old recording occupying the session after Resume.
#[tokio::test]
async fn pause_discards_then_resume_allows_a_fresh_recording() {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    let (core, encoder) = core("external-video-pause-resume").await;
    let paused = Arc::new(AtomicBool::new(false));
    let starts = Arc::new(AtomicUsize::new(0));
    let paused_for_helper = paused.clone();
    let starts_for_helper = starts.clone();
    let seen = super::test_support::answering(&core.shared, move |tool, args| {
        pause_answer(tool, args, &paused_for_helper, &starts_for_helper)
    });
    super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await
    .unwrap();
    core.shared
        .paused_tools
        .pause("s", crate::paused_tools::BROWSER, "Firefox");
    paused.store(true, Ordering::SeqCst);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !encoder.0.lock().unwrap().iter().any(|c| {
            matches!(c,
            VideoCommand::Cancel { recording } if recording == "video-test")
        }) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!encoder.0.lock().unwrap().iter().any(|c| matches!(c,
        VideoCommand::Finish { recording, .. } if recording == "video-test")));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !seen.lock().unwrap().iter().any(|(tool, args)| {
            tool == "chrome_execute"
                && args["action"] == "recording.video.stop"
                && args["args"]["discard"] == true
        }) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    paused.store(false, Ordering::SeqCst);
    core.shared
        .paused_tools
        .resume("s", Some(crate::paused_tools::BROWSER));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match super::read_consent::send(
                &core.shared,
                "s",
                "chrome_execute",
                request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
                None,
            )
            .await
            {
                Ok(_) => break,
                Err(error) if error.contains("already has a video") => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => panic!("fresh Start after Resume failed: {error}"),
            }
        }
    })
    .await
    .unwrap();
    let result = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.stop", json!({"tabId":7,"discard":true})),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        result.value()["structuredContent"]["data"]["state"],
        "discarded"
    );
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(tool, args)| tool == "chrome_execute"
                && args["action"] == "recording.video.stop"
                && args["args"]["discard"] == true)
    );
    core.shutdown().await;
}
