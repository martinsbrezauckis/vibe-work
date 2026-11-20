//! Adversarial contract tests for guarded external-browser video across chats, modes and input boundaries.
//! The mocked helper controls identities and screenshots; these tests do not claim native browser behavior.
use base64::Engine as _;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use vw_browser::recording::{VideoBridge, VideoCommand, VideoFuture, VideoReply};

#[derive(Default)]
pub(super) struct Encoder(pub(super) Mutex<Vec<VideoCommand>>);
impl VideoBridge for Encoder {
    fn send(&self, command: VideoCommand) -> VideoFuture<'_> {
        self.0.lock().unwrap().push(command.clone());
        Box::pin(async move {
            Ok(match command {
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

pub(super) fn identity() -> Value {
    json!({"profileId":"firefox:agent","browserSessionId":"browser-test","leaseEpoch":1,
        "policyRevision":1,"documentId":"doc-test","tabId":7,"recordingId":"video-test"})
}
pub(super) fn status() -> Value {
    json!({"structuredContent":{"profiles":[{"profileId":"firefox:agent","browserSessionId":"browser-test",
        "connected":true,"extensionCapabilities":["page.video.frames.v1"],
        "features":["recording.video.start","recording.video.stop","capture.screenshot"]}],
        "sessions":[{"sessionId":"t","profileId":"firefox:agent","state":"active",
        "leaseEpoch":1,"policyRevision":1,"target":{"tabId":7,"documentId":"doc-test"}}]}})
}
pub(super) fn start_result(seconds: u64) -> Value {
    let mut data = identity();
    data["state"] = json!("recording");
    data["appEncoder"] = json!(true);
    data["maxSeconds"] = json!(seconds);
    json!({"structuredContent":{"status":"completed","data":data}})
}
pub(super) fn stopped_result() -> Value {
    let mut data = identity();
    data["state"] = json!("discarded");
    data["appEncoder"] = json!(true);
    json!({"structuredContent":{"status":"completed","data":data}})
}
pub(super) fn saved_stop_result() -> Value {
    let mut data = identity();
    data["state"] = json!("stopped");
    data["appEncoder"] = json!(true);
    json!({"structuredContent":{"status":"completed","data":data}})
}
pub(super) fn screenshot(caps: &Value) -> Value {
    assert!(
        caps.as_array()
            .is_some_and(|rows| rows.contains(&json!("capture.screenshot")))
    );
    assert!(
        !caps
            .as_array()
            .is_some_and(|rows| rows.contains(&json!("auth.secrets")))
    );
    let mut data = json!({"mimeType":"image/png"});
    data["captureIdentity"] = identity();
    json!({"structuredContent":{"status":"completed","data":data},
        "content":[{"type":"image","mimeType":"image/png",
            "data":base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nmock")}]})
}
pub(super) async fn core(name: &str) -> (crate::Core, Arc<Encoder>) {
    let core = super::read_consent_tests::core(name).await;
    set_chat_folder(&core, "s");
    core.shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", "firefox:agent", None);
    let encoder = Arc::new(Encoder::default());
    core.set_browser_video_bridge(encoder.clone());
    (core, encoder)
}
pub(super) fn set_chat_folder(core: &crate::Core, chat: &str) {
    let folder = core
        .shared
        .instructions
        .own_home
        .parent()
        .unwrap()
        .join(format!("video-{chat}"));
    std::fs::create_dir_all(&folder).unwrap();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":chat,"cwd":folder},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
}
pub(super) fn request(action: &str, args: Value) -> Value {
    let mut request = json!({"sessionId":"t","action":action});
    request["args"] = args;
    request
}

// PREDICT: a mocked helper may accept an invalid duration; the app must reject it before Start has any effect.
#[tokio::test]
async fn invalid_duration_never_starts_helper_or_encoder() {
    let (core, encoder) = core("external-video-bad-duration").await;
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => status(),
        "chrome_execute" if args["action"] == "recording.video.start" => start_result(2),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            screenshot(&args["_approvedCapabilities"])
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => {
            json!({"structuredContent":{"status":"completed","data":{"state":"discarded"}}})
        }
        _ => panic!("unexpected {tool}: {args}"),
    });
    for seconds in [json!(0), json!(301), json!(-1), json!(1.5), json!("60")] {
        let answer = super::read_consent::send(
            &core.shared,
            "s",
            "chrome_execute",
            request(
                "recording.video.start",
                json!({"tabId":7,"maxSeconds":seconds}),
            ),
            None,
        )
        .await;
        assert!(
            answer.is_err(),
            "invalid maxSeconds {seconds} started video"
        );
    }
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, args)| tool == "chrome_execute"
                && args["action"] == "recording.video.start")
    );
    assert!(encoder.0.lock().unwrap().is_empty());
    core.shutdown().await;
}

// PREDICT: a Full-access chat could grant auth.secrets to a frame and bypass the protected-field scan.
#[tokio::test]
async fn full_access_video_frame_withholds_secret_capability() {
    let (core, encoder) = core("external-video-full-protected").await;
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => status(),
        "chrome_execute" if args["action"] == "recording.video.start" => start_result(2),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            assert!(
                !args["_approvedCapabilities"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("auth.secrets"))
            );
            json!({"structuredContent":{"status":"failed","error":{"code":"CAPABILITY_REQUIRED",
                "details":{"capability":"auth.secrets","operation":"capturing a page that shows a password, PIN or card value"}}}})
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => {
            json!({"structuredContent":{"status":"completed","data":{"state":"discarded"}}})
        }
        _ => panic!("unexpected {tool}: {args}"),
    });
    let answer = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await;
    // The refusal names the real reason instead of a generic capture failure.
    assert_eq!(
        answer.err().as_deref(),
        Some(super::external_video::PROTECTED_START)
    );
    {
        let calls = encoder.0.lock().unwrap();
        assert!(
            !calls
                .iter()
                .any(|c| matches!(c, VideoCommand::Frame { .. }))
        );
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, VideoCommand::Cancel { .. }))
        );
    }
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(tool, args)| tool == "chrome_execute" && args["action"] == "capture.screenshot")
    );
    core.shutdown().await;
}

// PREDICT: another chat may use the first chat's session ID to stop or save its WebM.
#[tokio::test]
async fn second_chat_cannot_stop_first_chats_recording() {
    let (core, encoder) = core("external-video-other-chat").await;
    super::test_support::chat(&core.shared, "other");
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => status(),
        "chrome_execute" if args["action"] == "recording.video.start" => start_result(2),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            screenshot(&args["_approvedCapabilities"])
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => stopped_result(),
        _ => panic!("unexpected {tool}: {args}"),
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
    let other = super::read_consent::send(
        &core.shared,
        "other",
        "chrome_execute",
        request("recording.video.stop", json!({"tabId":7,"discard":true})),
        None,
    )
    .await;
    assert!(other.is_err());
    let wrong_tab = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.stop", json!({"tabId":8,"discard":true})),
        None,
    )
    .await;
    assert!(wrong_tab.is_err());
    assert!(
        !seen.lock().unwrap().iter().any(
            |(tool, args)| tool == "chrome_execute" && args["action"] == "recording.video.stop"
        )
    );
    assert!(
        !encoder
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, VideoCommand::Finish { .. }))
    );
    let owner = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.stop", json!({"tabId":7,"discard":true})),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        owner.value()["structuredContent"]["data"]["state"],
        "discarded"
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

// PREDICT: a protected value appearing after the first safe frame could be forwarded into the safe-prefix WebM.
#[tokio::test]
async fn late_protected_field_refusal_saves_only_safe_prefix() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (core, encoder) = core("external-video-late-protected").await;
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    let captures = Arc::new(AtomicUsize::new(0));
    let checked = captures.clone();
    let seen = super::test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_status" => status(),
        "chrome_execute" if args["action"] == "recording.video.start" => start_result(2),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            let index = checked.fetch_add(1, Ordering::SeqCst);
            if index == 0 {
                screenshot(&args["_approvedCapabilities"])
            } else {
                assert!(
                    !args["_approvedCapabilities"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("auth.secrets"))
                );
                json!({"structuredContent":{"status":"failed","error":{"code":"CAPABILITY_REQUIRED",
                    "details":{"capability":"auth.secrets","operation":"capturing a page that shows a password, PIN or card value"}}}})
            }
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => saved_stop_result(),
        _ => panic!("unexpected {tool}: {args}"),
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
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while captures.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let stop = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request(
            "recording.video.stop",
            json!({"tabId":7,"filename":"late-protected.webm"}),
        ),
        None,
    )
    .await
    .unwrap();
    let data = &stop.value()["structuredContent"]["data"];
    assert_eq!(data["state"], "saved");
    assert_eq!(data["limit"], "protected_field");
    assert_eq!(data["frames"], 1);
    assert_eq!(data["file"]["path"], "late-protected.webm");
    {
        let calls = encoder.0.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|c| matches!(c, VideoCommand::Frame { .. }))
                .count(),
            1
        );
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, VideoCommand::Frame { png, .. }
        if png == b"\x89PNG\r\n\x1a\nmock"))
        );
        assert!(
            !calls
                .iter()
                .any(|c| matches!(c, VideoCommand::Cancel { .. }))
        );
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, VideoCommand::Finish { .. }))
        );
    }
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(tool, args)| tool == "chrome_execute"
                && args["action"] == "recording.video.stop"
                && args["args"]["discard"] != true)
    );
    core.shutdown().await;
}

// PREDICT: a mode downgrade could leave the worker and encoded frames alive after access revocation.
#[tokio::test]
async fn access_downgrade_cancels_external_video() {
    let (core, encoder) = core("external-video-downgrade").await;
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => status(),
        "chrome_session" if args["action"] == "stop" => {
            json!({"structuredContent":{"status":"completed"}})
        }
        "chrome_execute" if args["action"] == "recording.video.start" => start_result(2),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            screenshot(&args["_approvedCapabilities"])
        }
        _ => panic!("unexpected {tool}: {args}"),
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
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"accept-edits"}),
    );
    super::check::mode_changed(&core.shared, "s", Some("auto-approve"));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !encoder
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, VideoCommand::Cancel { .. }))
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !encoder
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, VideoCommand::Finish { .. }))
    );
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, args)| tool == "chrome_execute"
                && args["action"] == "recording.video.stop"
                && args["args"]["discard"] != true)
    );
    core.shutdown().await;
}

// PREDICT: two WebMs with omitted filenames could collide in the same chat folder.
