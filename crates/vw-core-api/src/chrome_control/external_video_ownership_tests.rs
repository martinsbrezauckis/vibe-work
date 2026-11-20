//! Which work owns the browser while a recording samples frames: a foreground action and a compiled page primitive
//! keep the operation until their reply, a frame delays a foreground call, and a queued Full request rechecks the mode.
use super::external_video_adversarial_tests::{core, request};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn foreground_action_owns_video_operation_until_helper_reply() {
    let (core, _) = core("external-video-foreground-lock").await;
    let _seen = core.shared.chrome_control.owner.answering_after(
        Duration::from_millis(500),
        |_, _| json!({"structuredContent":{"status":"completed","data":{}}}),
    );
    let operation = core.shared.chrome_control.external_video.operation("t");
    let before = core.shared.chrome_control.external_video.operation_calls();
    let shared = core.shared.clone();
    let foreground = tokio::spawn(async move {
        super::escalation::send(
            &shared,
            "s",
            "chrome_execute",
            request("page.read", json!({})),
            None,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while core.shared.chrome_control.external_video.operation_calls() == before {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        operation.try_lock().is_err(),
        "foreground action left frame operation lock free"
    );
    foreground.await.unwrap().unwrap();
    core.shutdown().await;
}

// PREDICT: a compiled page primitive leaves the same session's frame lock free while its helper reply is pending.
#[tokio::test]
async fn compiled_page_primitive_owns_video_operation_until_receipt() {
    let (core, _) = core("external-video-primitive-lock").await;
    let _seen = core.shared.chrome_control.owner.answering_after(Duration::from_millis(500), |tool, _| match tool {
        "chrome_status" => json!({"structuredContent":{"profiles":[{"profileId":"firefox:agent","connected":true,
            "extensionCapabilities":["transport.contract.v1"],"features":["page.read"]}],
            "sessions":[{"sessionId":"t","profileId":"firefox:agent","state":"active","leaseEpoch":1,
            "policyRevision":1,"target":{"tabId":7,"documentId":"doc-test"}}]}}),
        "owner.primitive" => json!({"status":"completed","data":{}}),
        "chrome_session" => json!({"structuredContent":{"status":"completed"}}),
        _ => panic!("unexpected helper method: {tool}"),
    });
    let operation = core.shared.chrome_control.external_video.operation("t");
    let shared = core.shared.clone();
    let primitive = tokio::spawn(async move {
        super::page_plan::dispatch(
            &shared,
            "s",
            &request("page.read", json!({"tabId":7})),
            None,
            json!([]),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        operation.try_lock().is_err(),
        "compiled page primitive left frame operation lock free"
    );
    primitive.await.unwrap().unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn frame_owned_operation_delays_foreground_helper_call() {
    let (core, _) = core("external-video-frame-priority").await;
    let seen = super::test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"status":"completed","data":{}}}),
    );
    let operation = core.shared.chrome_control.external_video.operation("t");
    let foreground = {
        let _guard = operation.lock().await;
        let shared = core.shared.clone();
        let foreground = tokio::spawn(async move {
            super::escalation::send(
                &shared,
                "s",
                "chrome_execute",
                request("page.read", json!({})),
                None,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|(tool, _)| tool == "chrome_execute"),
            "foreground reached helper while frame owned its session"
        );
        foreground
    };
    foreground.await.unwrap().unwrap();
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute")
    );
    core.shutdown().await;
}

// PREDICT: a Full-only request approved before waiting for a frame can reach the helper after downgrade.
#[tokio::test]
async fn queued_full_request_rechecks_mode_before_helper_dispatch() {
    let (core, _) = core("external-video-queued-downgrade").await;
    *core
        .shared
        .chrome_control
        .owner
        .firefox_pairing
        .lock()
        .unwrap() = (super::PairingState::WindowOpen, None);
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    let full_request = request("cookies.read", json!({}));
    assert!(
        super::check::authorize_read(&core.shared, "s", "chrome_execute", &full_request, None)
            .is_ok(),
        "fixture did not admit Full-only request"
    );
    let seen = super::test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"status":"completed","data":{}}}),
    );
    let operation = core.shared.chrome_control.external_video.operation("t");
    let before = core.shared.chrome_control.external_video.operation_calls();
    let queued = {
        let _guard = operation.lock().await;
        let shared = core.shared.clone();
        let queued = tokio::spawn(async move {
            super::escalation::send(
                &shared,
                "s",
                "chrome_execute",
                request("cookies.read", json!({})),
                None,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while core.shared.chrome_control.external_video.operation_calls() == before {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            !queued.is_finished(),
            "request finished before waiting for frame lock"
        );
        core.shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"ask"}),
        );
        queued
    };
    let result = queued.await.unwrap();
    assert!(
        result.is_err(),
        "Full-only request was admitted after Ask downgrade"
    );
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute"),
        "Full-only request reached helper after downgrade"
    );
    core.shutdown().await;
}
