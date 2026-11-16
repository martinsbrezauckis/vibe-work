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
