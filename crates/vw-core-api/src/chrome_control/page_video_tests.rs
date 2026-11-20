//! Page video routes to the helper only when the chat's browser reports page.video.v1; older extensions keep the
//! GIF alternative text and nothing is sent.
//!
//! PREDICT (before the change): recording.video.start always returns the "unavailable" text, so the capable
//! profile never reaches the helper.
use serde_json::{Value, json};

fn status(capabilities: &[&str]) -> Value {
    json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,
        "extensionCapabilities":capabilities,"features":["recording.video.start","recording.video.stop"]}],
        "sessions":[{"sessionId":"t","profileId":"chrome:p"}]}})
}

#[tokio::test]
async fn video_reaches_the_helper_when_the_extension_records_video() {
    let core = super::read_consent_tests::core("page-video-capable").await;
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => status(&["transport.contract.v1", "page.video.v1"]),
        "chrome_execute" => {
            assert_eq!(args["action"], "recording.video.start");
            assert!(
                args["_approvedCapabilities"]
                    .as_array()
                    .is_some_and(|caps| caps.contains(&json!("capture.screenshot"))),
                "{args}"
            );
            json!({"structuredContent":{"status":"completed","data":{"state":"recording","tabId":7}}})
        }
        _ => panic!("unexpected tool {tool}"),
    });
    let request = json!({"sessionId":"t","action":"recording.video.start","args":{"tabId":7,"maxSeconds":20}});
    let answer = super::read_consent::send(&core.shared, "s", "chrome_execute", request, None)
        .await
        .unwrap();
    assert_eq!(
        answer.value()["structuredContent"]["data"]["state"],
        "recording"
    );
    core.shutdown().await;
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "chrome_execute")
            .count(),
        1
    );
}

#[tokio::test]
async fn older_extensions_keep_the_gif_alternative_and_send_nothing() {
    let core = super::read_consent_tests::core("page-video-old").await;
    let seen = super::test_support::answering(&core.shared, |tool, _| match tool {
        "chrome_status" => status(&["transport.contract.v1"]),
        _ => panic!("unexpected tool {tool}"),
    });
    for action in ["recording.video.start", "recording.video.stop"] {
        let request = json!({"sessionId":"t","action":action,"args":{"tabId":7}});
        let Err(error) =
            super::read_consent::send(&core.shared, "s", "chrome_execute", request, None).await
        else {
            panic!("{action} must not reach the helper on an older extension");
        };
        assert!(error.contains("recording.start"), "{error}");
        assert!(error.contains("update"), "{error}");
    }
    core.shutdown().await;
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "chrome_execute")
    );
}

// PREDICT: a Firefox or Safari profile with the guarded-frame capability is refused before
// helper admission, so its app encoder never receives a start request.
#[derive(Default)]
struct Encoder(std::sync::Mutex<Vec<vw_browser::recording::VideoCommand>>);
impl vw_browser::recording::VideoBridge for Encoder {
    fn send(
        &self,
        command: vw_browser::recording::VideoCommand,
    ) -> vw_browser::recording::VideoFuture<'_> {
        self.0.lock().unwrap().push(command.clone());
        Box::pin(async move {
            Ok(match command {
                vw_browser::recording::VideoCommand::Start { .. } => {
                    vw_browser::recording::VideoReply::Ready
                }
                vw_browser::recording::VideoCommand::Frame { .. }
                | vw_browser::recording::VideoCommand::Cancel { .. } => {
                    vw_browser::recording::VideoReply::Added
                }
                vw_browser::recording::VideoCommand::Finish { .. } => {
                    vw_browser::recording::VideoReply::Finished {
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
async fn guarded_external_video_start_reaches_the_helper() {
    use base64::Engine as _;
    let core = super::read_consent_tests::core("page-video-guarded-external").await;
    *core
        .shared
        .chrome_control
        .owner
        .firefox_pairing
        .lock()
        .unwrap() = (super::PairingState::WindowOpen, None);
    core.shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap()
        .insert("s".into(), "firefox:agent".into());
    core.shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", "firefox:agent", None);
    let encoder = std::sync::Arc::new(Encoder::default());
    core.set_browser_video_bridge(encoder.clone());
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => {
            json!({"structuredContent":{"profiles":[{"profileId":"firefox:agent","browserSessionId":"browser-test","connected":true,
            "extensionCapabilities":["firefox.page.v1","page.video.frames.v1"],
            "features":["recording.video.start","recording.video.stop","capture.screenshot"]}],
            "sessions":[{"sessionId":"t","profileId":"firefox:agent","state":"active","leaseEpoch":1,"policyRevision":1,"target":{"tabId":7,"documentId":"doc-test"}}]}})
        }
        "chrome_execute" if args["action"] == "recording.video.start" => {
            json!({"structuredContent":{"status":"completed","data":{"state":"recording","appEncoder":true,"tabId":7,"maxSeconds":2,
            "recordingId":"video-test","profileId":"firefox:agent","browserSessionId":"browser-test","leaseEpoch":1,"policyRevision":1,"documentId":"doc-test"}}})
        }
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            assert!(
                !args["_approvedCapabilities"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("auth.secrets"))
            );
            json!({"structuredContent":{"status":"completed","data":{"mimeType":"image/png","captureIdentity":{"profileId":"firefox:agent","browserSessionId":"browser-test","leaseEpoch":1,"policyRevision":1,"documentId":"doc-test","tabId":7,"recordingId":"video-test"}}},"content":[{"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nmock")}]})
        }
        _ => panic!("unexpected tool {tool}"),
    });
    let request =
        json!({"sessionId":"t","action":"recording.video.start","args":{"tabId":7,"maxSeconds":2}});
    let answer = super::read_consent::send(&core.shared, "s", "chrome_execute", request, None)
        .await
        .unwrap();
    assert_eq!(
        answer.value()["structuredContent"]["data"]["state"],
        "recording"
    );
    assert!(
        encoder
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, vw_browser::recording::VideoCommand::Frame { .. }))
    );
    core.shutdown().await;
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute")
    );
}
