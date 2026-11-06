//! Paired browser reconnects through the ordinary gateway; deterministic helper flaps, no real desktop.
use super::{agent_browser::Browser, edge_agent_tests::fixture, gateway, owner};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

// The owner also logs background status calls; match all requested fields, allowing gateway metadata.
#[allow(
    clippy::unwrap_used,
    reason = "test call logs and request objects must be valid"
)]
fn call_count(seen: &Mutex<Vec<(String, Value)>>, tool: &str, request: &Value) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(name, args)| {
            name == tool
                && request
                    .as_object()
                    .unwrap()
                    .iter()
                    .all(|(key, value)| args.get(key) == Some(value))
        })
        .count()
}

#[tokio::test]
async fn status_observes_a_reconnected_extension_before_repeating_an_old_refusal() {
    let core = fixture("browser-reconnected-status", json!([])).await;
    core.shared.chrome_control.mark_paired();
    let weak = Arc::downgrade(&core.shared);
    let calls = core.shared.chrome_control.owner.answering(move |tool, _| {
        assert_eq!(tool, "chrome_status");
        let reply = json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,"extensionVersion":"0.1.5"}],"sessions":[]}});
        owner::update_pairing(&weak, &reply);
        reply
    });
    let answer = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "read the current connection once"
    );
    assert_eq!(
        answer["vwMcpResult"]["structuredContent"]["untrustedPageContent"]["addon"]["stateCode"],
        "connected",
        "{answer}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_session_still_reconnecting_offers_a_plain_retry_in_its_browser() {
    for (browser, profile) in [(Browser::Chrome, "chrome:p"), (Browser::Edge, "edge:p")] {
        let core = fixture(
            "browser-session-not-ready",
            json!([{"profileId":profile,"connected":true}]),
        )
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let seen = core.shared.chrome_control.owner.answering(move |tool, _| {
            if tool == "chrome_session" && count.fetch_add(1, Ordering::SeqCst) == 0 {
                let mut reply = owner::failure("Ask the person to restart the browser", "SESSION_NOT_READY");
                reply["structuredContent"]["retryable"] = json!(false);
                reply
            } else {
                json!({"structuredContent":{"sessionId":"ready","profileId":profile,"state":"active"}})
            }
        });
        let args = json!({"action":"open","profileId":profile});
        let reply = gateway::call(&core.shared, "s", "chrome_session", args.clone())
            .await
            .unwrap();
        let body = &reply["vwMcpResult"]["structuredContent"];
        assert_eq!(body["retryable"], true, "{reply}");
        assert!(
            reply.to_string().contains(&format!(
                "{} is reconnecting; try again in a few seconds",
                browser.name()
            )),
            "{reply}"
        );
        assert!(!reply.to_string().contains("Ask the person"), "{reply}");
        assert_eq!(
            call_count(&seen, "chrome_session", &args),
            1,
            "no automatic replay"
        );
        let next = gateway::call(&core.shared, "s", "chrome_session", args)
            .await
            .unwrap();
        assert_eq!(
            next["vwMcpResult"]["structuredContent"]["sessionId"],
            "ready"
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn paired_navigation_flaps_wait_and_execute_once_without_an_owner_card() {
    for (browser, profile) in [(Browser::Chrome, "chrome:p"), (Browser::Edge, "edge:p")] {
        let core = fixture(
            &format!("browser-reconnect-{}", browser.id()),
            json!([{"profileId":profile,"connected":true}]),
        )
        .await;
        gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .unwrap();
        let chrome = &core.shared.chrome_control;
        // A registered browser was paired, then briefly reports connecting while its native port rebinds.
        let pairing = if browser == Browser::Edge {
            &chrome.owner.edge_pairing
        } else {
            &chrome.owner.pairing
        };
        pairing.lock().unwrap().0 = crate::chrome_control::PairingState::Connecting;
        let weak = Arc::downgrade(&core.shared);
        let polls = Arc::new(AtomicUsize::new(0));
        let sampled = polls.clone();
        let seen = core.shared.chrome_control.owner.answering( move |tool, _| {
            let result = if tool == "chrome_status" {
                let connected = sampled.fetch_add(1, Ordering::SeqCst) > 0;
                json!({"structuredContent":{"profiles":[{"profileId":profile,"connected":connected}],"sessions":[{"sessionId":"task","state":"active","profileId":profile}]}})
            } else if sampled.load(Ordering::SeqCst) < 2 {
                owner::failure("Native port is reconnecting", "PROFILE_NOT_CONNECTED")
            } else {
                json!({"structuredContent":{"status":"completed","data":{}}})
            };
            owner::update_pairing(&weak, &result);
            result
        });
        let args = json!({"sessionId":"task","action":"page.read","args":{}});
        let result = gateway::call(&core.shared, "s", "chrome_execute", args.clone())
            .await
            .unwrap();
        assert_eq!(
            result["vwMcpResult"]["structuredContent"]["status"],
            "completed",
            "{}: {result}",
            browser.name()
        );
        assert!(!result.to_string().contains("OWNER_REQUIRED"), "{result}");
        let sent =
            json!({"sessionId":"task","action":"page.read","args":{"maxBytes":2000,"limit":8}});
        assert_eq!(call_count(&seen, "chrome_execute", &sent), 1);
        core.shutdown().await;
    }
}

#[tokio::test]
async fn paired_reconnect_timeout_gives_the_cards_recovery_step_without_resending() {
    let core = fixture(
        "browser-reconnect-timeout",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    core.shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap()
        .0 = crate::chrome_control::PairingState::Connecting;
    let seen =
        core.shared.chrome_control.owner.answering(|_, _| {
            owner::failure("Native port is reconnecting", "PROFILE_NOT_CONNECTED")
        });
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    let result = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"task","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    let text = result.to_string();
    let card = Browser::Edge.addon(&core.shared);
    assert_eq!(card.label, "Turn on in Edge");
    assert!(card.action.is_none(), "recovery must not offer enrollment");
    assert!(text.contains(&card.instruction), "{result}");
    assert!(
        text.contains("Extensions page") && text.contains("restart Edge"),
        "{result}"
    );
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["error"]["code"],
        "EXTENSION_UNRESPONSIVE"
    );
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["retryable"],
        true
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool == "chrome_status")
    );
    assert!(
        !text.contains("OWNER_REQUIRED") && !text.contains("addon_suggest"),
        "{result}"
    );
    assert!(
        text.contains("Retry this step in a few seconds."),
        "{result}"
    );
    // Tokio's millisecond timer wheel rounds the five-second deadline up by one tick.
    assert!(
        started.elapsed() <= std::time::Duration::from_millis(5_010),
        "{:?}",
        started.elapsed()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn another_chat_is_refused_before_waiting_for_a_reconnect() {
    let core = fixture(
        "browser-reconnect-other-chat",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"other"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.shared.access.observe_request(
        "session/agent/update",
        "other",
        &json!({"agentName":"auto-approve"}),
    );
    core.shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap()
        .0 = crate::chrome_control::PairingState::Connecting;
    let weak = Arc::downgrade(&core.shared);
    let seen = core.shared.chrome_control.owner.answering(move |_, _| {
        let status =
            json!({"structuredContent":{"profiles":[{"profileId":"edge:p","connected":true}]}});
        owner::update_pairing(&weak, &status);
        status
    });
    let reason = gateway::call(
        &core.shared,
        "other",
        "chrome_execute",
        json!({"sessionId":"task","action":"page.read","args":{}}),
    )
    .await
    .unwrap_err();
    assert!(reason.contains("another chat"), "{reason}");
    assert!(
        seen.lock().unwrap().iter().all(|(tool, args)| {
            tool == "chrome_status" && args.get("_netPolicyChange").is_some()
        }),
        "another chat cannot start a reconnect wait"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_ended_session_keeps_the_plain_retry_receipt_when_reopening_cannot_reconnect() {
    let core = fixture(
        "browser-reconnect-ended-session",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    super::end_turn(&core.shared, "s");
    core.shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap()
        .0 = crate::chrome_control::PairingState::Connecting;
    core.shared
        .chrome_control
        .owner
        .answering(|_, _| owner::failure("Native port is reconnecting", "PROFILE_NOT_CONNECTED"));
    tokio::time::pause();
    let result = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"task","action":"page.read","args":{}}),
    )
    .await;
    assert!(
        result.is_ok(),
        "reopening must retain the retry receipt: {result:?}"
    );
    let answer = result.unwrap();
    assert_eq!(
        answer["vwMcpResult"]["structuredContent"]["error"]["code"],
        "EXTENSION_UNRESPONSIVE"
    );
    assert_eq!(
        answer["vwMcpResult"]["structuredContent"]["retryable"],
        true
    );
    assert!(!answer.to_string().contains("addon_suggest"));
    core.shutdown().await;
}

#[tokio::test]
async fn a_disconnect_during_an_action_is_not_replayed_or_replaced_by_pairing_instructions() {
    for (browser, profile) in [(Browser::Chrome, "chrome:p"), (Browser::Edge, "edge:p")] {
        let core = fixture(
            &format!("browser-reconnect-after-send-{}", browser.id()),
            json!([{"profileId":profile,"connected":true}]),
        )
        .await;
        gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .unwrap();
        let weak = Arc::downgrade(&core.shared);
        let seen = core.shared.chrome_control.owner.answering(move |_, _| {
            let shared = weak.upgrade().unwrap();
            let pairing = if browser == Browser::Chrome {
                &shared.chrome_control.owner.pairing
            } else {
                &shared.chrome_control.owner.edge_pairing
            };
            pairing.lock().unwrap().0 = crate::chrome_control::PairingState::Connecting;
            owner::failure("Native port is reconnecting", "PROFILE_NOT_CONNECTED")
        });
        let args = json!({"sessionId":"task","action":"page.read","args":{}});
        let result = gateway::call(&core.shared, "s", "chrome_execute", args.clone())
            .await
            .unwrap();
        assert_eq!(
            result["vwMcpResult"]["structuredContent"]["error"]["code"], "SESSION_NOT_READY",
            "{result}"
        );
        assert_eq!(
            result["vwMcpResult"]["structuredContent"]["retryable"],
            true
        );
        // Exercise the background switch delivery that can race the action under suite load.
        super::send_local_network(&core.shared).await;
        let sent =
            json!({"sessionId":"task","action":"page.read","args":{"maxBytes":2000,"limit":8}});
        assert_eq!(
            call_count(&seen, "chrome_execute", &sent),
            1,
            "an action already sent is never retried"
        );
        assert!(!result.to_string().contains("addon_suggest"), "{result}");
        core.shutdown().await;
    }
}

#[tokio::test]
async fn an_unknown_action_outcome_keeps_its_original_receipt_during_reconnect() {
    let core = fixture(
        "browser-reconnect-unknown-outcome",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    let weak = Arc::downgrade(&core.shared);
    let seen = core.shared.chrome_control.owner.answering(move |_, _| {
        weak.upgrade().unwrap().chrome_control.owner.edge_pairing.lock().unwrap().0 = crate::chrome_control::PairingState::Connecting;
        json!({"structuredContent":{"status":"unknown","error":{"code":"ACTION_TIMEOUT","message":"Action outcome unknown"}}})
    });
    let args = json!({"sessionId":"task","action":"page.click","args":{"ref":"button"}});
    let result = gateway::call(&core.shared, "s", "chrome_execute", args.clone())
        .await
        .unwrap();
    let body = &result["vwMcpResult"]["structuredContent"];
    assert_eq!(body["status"], "unknown", "{result}");
    assert_eq!(
        body["untrustedPageContent"]["error"]["code"], "ACTION_TIMEOUT",
        "{result}"
    );
    assert_ne!(body["retryable"], true);
    assert_eq!(call_count(&seen, "chrome_execute", &args), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn a_person_stop_received_during_reconnect_prevents_the_action() {
    let core = fixture(
        "browser-reconnect-stop",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    core.shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap()
        .0 = crate::chrome_control::PairingState::Connecting;
    let weak = Arc::downgrade(&core.shared);
    let seen = core.shared.chrome_control.owner.answering(move |_, _| {
        let shared = weak.upgrade().unwrap();
        let _ = shared
            .chrome_control
            .person_stopped(&[super::person_stop::Stopped {
                chat: "s".into(),
                browser: Browser::Edge,
                turn: None,
            }]);
        json!({"structuredContent":{"profiles":[]}})
    });
    let reason = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"task","action":"page.read","args":{}}),
    )
    .await
    .unwrap_err();
    assert!(
        reason.contains(&format!("you pressed Stop in {}", Browser::Edge.name())),
        "{reason}"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool == "chrome_status")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn connecting_before_the_first_pairing_keeps_the_owner_diagnosis() {
    for (browser, profile) in [(Browser::Chrome, "chrome:p"), (Browser::Edge, "edge:p")] {
        let core = fixture(
            &format!("browser-first-pairing-{}", browser.id()),
            json!([{"profileId":profile,"connected":true}]),
        )
        .await;
        let chrome = &core.shared.chrome_control;
        chrome.paired_once.store(false, Ordering::SeqCst);
        chrome.edge_paired_once.store(false, Ordering::SeqCst);
        let pairing = if browser == Browser::Chrome {
            &chrome.owner.pairing
        } else {
            &chrome.owner.edge_pairing
        };
        pairing.lock().unwrap().0 = crate::chrome_control::PairingState::Connecting;
        chrome
            .owner
            .answering(|_, _| owner::failure("Not registered yet", "PROFILE_NOT_CONNECTED"));
        let result = gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .unwrap();
        assert!(
            result.to_string().contains("Ask the person")
                && result.to_string().contains("OWNER_REQUIRED"),
            "{result}"
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn a_new_pairing_request_is_not_hidden_by_old_connection_history() {
    let core = fixture(
        "browser-reconnect-new-pairing",
        json!([{"profileId":"chrome:p","connected":true}]),
    )
    .await;
    let chrome = &core.shared.chrome_control;
    chrome.owner.enroll.lock().unwrap().observe_state(
        Some("waiting"),
        &json!({}),
        super::enroll_browser::Browser::Chrome,
    );
    chrome.owner.pairing.lock().unwrap().0 = crate::chrome_control::PairingState::Connecting;
    chrome
        .owner
        .answering(|_, _| owner::failure("Not registered yet", "PROFILE_NOT_CONNECTED"));
    tokio::time::pause();
    let result = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:p"}),
    )
    .await
    .unwrap();
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["error"]["code"], "OWNER_REQUIRED",
        "{result}"
    );
    assert!(result.to_string().contains("Ask the person"), "{result}");
    core.shutdown().await;
}
