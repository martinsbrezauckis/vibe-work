//! External receipt normalization through the real model boundary; no desktop, helper process or page injection.
use super::*;
use crate::chrome_control::{output_cache, output_tests, owner::Answer};
use serde_json::json;
use vw_preview::action_tracking::ActionKind;

#[tokio::test]
async fn external_receipts_use_host_binding_and_never_retain_typed_values() {
    let core = output_tests::core("action-receipts").await;
    for (profile, browser) in [
        ("legacy-profile", "chrome"),
        ("chrome:one", "chrome"),
        ("edge:one", "edge"),
        ("safari:one", "safari"),
        ("firefox:one", "firefox"),
    ] {
        core.shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .open("t", "s", "active", profile, None);
        let mut events = core.shared.action_tracker().subscribe();
        let receipt = output_tests::mcp(json!({
            "operationId":"input", "sessionId":"t", "action":"page.type", "status":"completed",
            "profileId":"forged", "data":{"insertedChars":5, "trustedInput":true,
                "target":{"ref":"field", "value":"secret text", "resultObserved":true},
                "navigation":{"occurred":false}, "observedForMs":312}
        }));
        let request =
            json!({"sessionId":"t","action":"page.type","args":{"tabId":7,"text":"secret text"}});
        let output = output_cache::for_model(
            &core.shared,
            "s",
            &request,
            Answer::Helper(receipt),
            "chrome_execute",
            false,
        );
        assert!(output["vwMcpResult"]["structuredContent"].is_object());
        let event = events
            .try_recv()
            .expect("completed external input must enter the shared action timeline");
        assert_eq!(event.kind, ActionKind::Type);
        assert_eq!(event.session_id, "s");
        assert_eq!(serde_json::to_value(event.source.browser).unwrap(), browser);
        assert_eq!(event.source.profile_id.as_deref(), Some(profile));
        assert_eq!(event.source.browser_session_id.as_deref(), Some("t"));
        assert_eq!(event.source.view_id.as_deref(), Some("7"));
        assert_eq!(
            event.geometry.normalized_point(),
            None,
            "older receipts cannot invent a marker point"
        );
        let encoded = serde_json::to_string(&event).unwrap();
        assert!(!encoded.contains("secret text"));
        assert!(!encoded.contains("forged"));
    }
    core.shutdown().await;
}

#[tokio::test]
async fn failures_foreign_sessions_and_read_replays_do_not_publish_input() {
    let core = output_tests::core("action-refusals").await;
    core.shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", "chrome:one", None);
    let mut events = core.shared.action_tracker().subscribe();
    for (chat, action, status, is_error) in [
        ("s", "page.click", "unknown", false),
        ("s", "page.click", "failed", false),
        ("other", "page.click", "completed", false),
        ("s", "page.read", "completed", false),
        ("s", "page.click", "completed", true),
    ] {
        let request = json!({"sessionId":"t","action":action,"args":{}});
        let result = json!({"isError":is_error,"structuredContent":{"sessionId":"t","action":action,"status":status}});
        record(&core.shared, chat, &request, &result);
        assert!(events.try_recv().is_err());
    }
    core.shutdown().await;
}

#[tokio::test]
async fn coordinate_receipts_keep_known_points_without_inventing_viewport_or_frame_offsets() {
    use vw_preview::action_tracking::ActionPoint;
    let core = output_tests::core("action-coordinates").await;
    core.shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", "chrome:one", None);
    let mut events = core.shared.action_tracker().subscribe();
    for frame in [0, 2] {
        let request = json!({"sessionId":"t","action":"page.click","args":{"tabId":9,"frameId":frame,"x":50,"y":75}});
        let result = output_tests::mcp(
            json!({"sessionId":"t","action":"page.click","status":"completed","data":{"tabId":9,"documentId":"document"}}),
        );
        record(&core.shared, "s", &request, &result);
        let event = events.try_recv().unwrap();
        assert_eq!(
            event.geometry.point,
            (frame == 0).then_some(ActionPoint { x: 50.0, y: 75.0 })
        );
        assert!(event.geometry.target_box.is_none());
        assert!(event.geometry.viewport.is_none());
        assert!(event.geometry.normalized_point().is_none());
    }
    core.shutdown().await;
}
