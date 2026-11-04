//! Browser identity after Stop and pairing history after closing Edge. Uses the owned fake helper, never a desktop.
use super::{AddonState, commands, edge_agent_tests::fixture, gateway, owner, store};
use serde_json::json;
use std::sync::Arc;

const CHROME: &str = "12345678-1234-1234-1234-123456789abc";

async fn stopped_session(browser: &str, profile: &str, other: &str) {
    let core = fixture(
        &format!("browser-stop-{browser}"),
        json!([{"profileId":CHROME,"connected":true},{"profileId":"edge:p","connected":true}]),
    )
    .await;
    let shared = &core.shared;
    gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
    .unwrap();
    owner::update_work_for_test(
        shared,
        &json!({"structuredContent":{"sessions":[{"sessionId":"task","state":"stopped"}]}}),
    );
    let expected = format!(
        "Not run: you pressed Stop in Your {browser}. The turn is stopped; do not call browser tools again in this turn."
    );
    for reset in [false, true] {
        if reset {
            shared.chrome_control.reset();
        }
        for (tool, args) in [
            (
                "chrome_execute",
                json!({"sessionId":"task","action":"page.read","args":{}}),
            ),
            (
                "chrome_session",
                json!({"action":"open","profileId":profile}),
            ),
        ] {
            assert_eq!(
                gateway::call(shared, "s", tool, args).await.unwrap_err(),
                expected,
                "{tool}, reset={reset}"
            );
        }
    }
    // The person's Stop holds only the browser they stopped, including across a helper reset.
    assert!(
        gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":other})
        )
        .await
        .is_ok()
    );
    super::end_turn(shared, "s");
    assert!(
        gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":other})
        )
        .await
        .is_ok()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn stop_keeps_the_edge_sessions_browser_through_retry_and_reset() {
    stopped_session("Edge", "edge:p", CHROME).await;
}

#[tokio::test]
async fn stop_keeps_the_chrome_sessions_browser_through_retry_and_reset() {
    stopped_session("Chrome", CHROME, "edge:p").await;
}

/// The helper may report stopping before replying to the agent's own Stop on either browser.
async fn agent_stop_then_open(profile: &str) {
    let core = fixture(
        &format!("browser-agent-stop-{profile}"),
        json!([{"profileId":CHROME,"connected":true},{"profileId":"edge:p","connected":true}]),
    )
    .await;
    let shared = &core.shared;
    gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
    .unwrap();
    let weak = Arc::downgrade(shared);
    shared.chrome_control.owner.answering(move |tool, args| {
            assert_eq!(tool, "chrome_session");
            if args["action"] == "stop" {
                let shared = weak.upgrade().unwrap();
                for state in ["stopping", "stopped"] {
                    owner::update_work_for_test(
                        &shared,
                        &json!({"structuredContent":{"sessions":[{"sessionId":"task","state":state}]}}),
                    );
                }
                json!({"structuredContent":{"sessionId":"task","state":"stopped"}})
            } else {
                json!({"structuredContent":{"sessionId":"new-task","profileId":args["profileId"],"state":"active"}})
            }
        });
    gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"stop","sessionId":"task"}),
    )
    .await
    .unwrap();
    let opened = gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
    .unwrap_or_else(|reason| panic!("agent stop blocked {profile}: {reason}"));
    assert_eq!(
        opened["vwMcpResult"]["structuredContent"]["sessionId"],
        "new-task"
    );
    assert_eq!(
        opened["vwMcpResult"]["structuredContent"]["profileId"],
        profile
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_agent_stop_allows_a_new_chrome_session_in_the_same_turn() {
    agent_stop_then_open(CHROME).await;
}

#[tokio::test]
async fn an_agent_stop_allows_a_new_edge_session_in_the_same_turn() {
    agent_stop_then_open("edge:p").await;
}

#[tokio::test]
async fn paired_edge_closed_has_a_calm_state_and_an_open_browser_instruction() {
    let core = fixture(
        "edge-closed-after-pairing",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    owner::update_pairing(
        &Arc::downgrade(&core.shared),
        &json!({"structuredContent":{"profiles":[{"profileId":"edge:p","connected":false}]}}),
    );
    let view = commands::view(&core.shared);
    let addon = view.edge.unwrap().addon.unwrap();
    assert_eq!(addon.state, AddonState::ChromeClosed, "{addon:?}");
    assert_eq!(addon.label, "Connected when Edge is open");
    assert_eq!(addon.action, None);
    assert!(
        addon
            .instruction
            .contains("Ask the person to open Microsoft Edge")
    );
    assert!(store::paired_once(
        &core.shared.chrome_control.dir().unwrap().join("edge")
    ));
    let reply = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    let text = reply.to_string();
    assert!(
        text.contains("Ask the person to open Microsoft Edge"),
        "{text}"
    );
    assert!(!text.contains("Connect Edge"), "{text}");
    core.shutdown().await;
    let restarted = fixture(
        "edge-closed-after-pairing",
        json!([{"profileId":"edge:p","connected":false}]),
    )
    .await;
    assert_eq!(
        commands::view(&restarted.shared)
            .edge
            .unwrap()
            .addon
            .unwrap()
            .state,
        AddonState::ChromeClosed
    );
    restarted.shutdown().await;
}

#[tokio::test]
async fn paired_edge_removed_from_status_is_closed_instead_of_still_connected() {
    let core = fixture(
        "edge-closed-empty-status",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    owner::update_pairing(
        &Arc::downgrade(&core.shared),
        &json!({"structuredContent":{"profiles":[]}}),
    );
    let addon = commands::view(&core.shared).edge.unwrap().addon.unwrap();
    assert_eq!(addon.state, AddonState::ChromeClosed, "{addon:?}");
    core.shutdown().await;
}

#[tokio::test]
async fn a_revoked_edge_pairing_no_longer_uses_the_closed_browser_state() {
    for code in ["OWNER_CONFLICT", "OWNER_APPROVAL_REQUIRED"] {
        let core = fixture(
            &format!("edge-pairing-revoked-{code}"),
            json!([{"profileId":"edge:p","connected":true}]),
        )
        .await;
        owner::update_pairing(
            &Arc::downgrade(&core.shared),
            &json!({"structuredContent":{"profileId":"edge:p","error":{"code":code}}}),
        );
        assert!(!store::paired_once(
            &core.shared.chrome_control.dir().unwrap().join("edge")
        ));
        owner::update_pairing(
            &Arc::downgrade(&core.shared),
            &json!({"structuredContent":{"profiles":[{"profileId":"edge:p","connected":false}]}}),
        );
        assert_eq!(
            commands::view(&core.shared)
                .edge
                .unwrap()
                .addon
                .unwrap()
                .state,
            if code == "OWNER_CONFLICT" {
                AddonState::OtherOwner
            } else {
                AddonState::NotConnected
            }
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn chrome_pairing_does_not_count_as_an_earlier_edge_pairing() {
    let core = fixture(
        "edge-never-paired",
        json!([{"profileId":CHROME,"connected":true},{"profileId":"edge:p","connected":false}]),
    )
    .await;
    let addon = commands::view(&core.shared).edge.unwrap().addon.unwrap();
    assert_eq!(addon.state, AddonState::NotConnected);
    core.shutdown().await;
}
