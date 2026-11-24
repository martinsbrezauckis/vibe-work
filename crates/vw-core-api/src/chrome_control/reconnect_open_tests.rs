//! Opening a session while a browser pairs or reconnects: a still-reconnecting session offers one plain retry, a
//! first or new pairing keeps the owner diagnosis, and a repeated unconfirmed open while connected asks the person.
use super::{
    agent_browser::Browser, edge_agent_tests::fixture, gateway, owner, reconnect_tests::call_count,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn a_session_still_reconnecting_offers_a_plain_retry_in_its_browser() {
    for (browser, profile) in [(Browser::Chrome, "chrome:p"), (Browser::Edge, "edge:p")] {
        let core = fixture(
            &format!("browser-session-not-ready-{profile}"),
            json!([{"profileId":profile,"connected":true}]),
        )
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let seen = core.shared.chrome_control.owner.answering(move |tool, _| {
            if tool == "chrome_session" && count.fetch_add(1, Ordering::SeqCst) == 0 {
                let mut reply = owner::failure("Ask the user to restart the browser", "SESSION_NOT_READY");
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
        assert!(!reply.to_string().contains("Ask the user"), "{reply}");
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
            result.to_string().contains("Ask the user")
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
    assert!(result.to_string().contains("Ask the user"), "{result}");
    core.shutdown().await;
}

// PREDICT (before the fix): every open the helper answers SESSION_NOT_READY while the browser stays connected was
// rewritten as "is reconnecting; try again in a few seconds" (retryable), and the refusal registered a reconnect
// wait that the next status poll released at once as "is connected again". The agent then looped for minutes on a
// browser whose card and chrome_status said connected (an extension holding every start, e.g. after an app restart).
// Expected: the first answer still offers the plain retry (a port that just rebound); a repeated one while connected
// says the browser did not confirm the session and asks the person, is not retryable, and never registers a wait.
#[tokio::test]
async fn a_repeated_unconfirmed_open_while_connected_asks_the_person_and_never_reports_a_reconnect()
{
    for (browser, profile) in [(Browser::Chrome, "chrome:p"), (Browser::Edge, "edge:p")] {
        let core = fixture(
            &format!("browser-open-unconfirmed-{profile}"),
            json!([{"profileId":profile,"connected":true}]),
        )
        .await;
        let _ = core.shared.store.load(json!({"session":{"id":"s","status":{"activeTurnId":"t1"}},"history":[],"turns":[{"id":"t1","status":"in_progress"}]}), vw_session::LoadKind::New);
        let ready = Arc::new(AtomicUsize::new(0));
        let opened = ready.clone();
        core.shared.chrome_control.owner.answering(move |tool, _| {
            if tool == "chrome_session" && opened.load(Ordering::SeqCst) == 0 {
                let mut reply = owner::failure("The browser did not confirm the session", "SESSION_NOT_READY");
                reply["structuredContent"]["retryable"] = json!(false);
                reply
            } else if tool == "chrome_session" {
                json!({"structuredContent":{"sessionId":"ready","profileId":profile,"state":"active"}})
            } else {
                json!({"structuredContent":{"profiles":[{"profileId":profile,"connected":true}],"sessions":[]}})
            }
        });
        let args = json!({"action":"open","profileId":profile});
        let first = gateway::call(&core.shared, "s", "chrome_session", args.clone())
            .await
            .unwrap();
        assert_eq!(
            first["vwMcpResult"]["structuredContent"]["retryable"], true,
            "{first}"
        );
        let second = gateway::call(&core.shared, "s", "chrome_session", args.clone())
            .await
            .unwrap();
        let body = &second["vwMcpResult"]["structuredContent"];
        assert_eq!(body["error"]["code"], "SESSION_NOT_READY", "{second}");
        assert_eq!(body["retryable"], false, "{second}");
        let text = second.to_string();
        assert!(!text.contains("reconnecting"), "{text}");
        assert!(
            text.contains("did not confirm")
                && text.contains("toolbar")
                && text.contains("Settings")
                && text.contains("Connection")
                && text.contains("Check connection"),
            "{text}"
        );
        assert_ne!(body["addon"]["stateCode"], "connecting", "{second}");
        let _ = gateway::call(&core.shared, "s", "chrome_status", json!({})).await;
        assert!(
            super::connection_notice::take(&core.shared).is_empty(),
            "{} never reports a reconnect for a browser that stayed connected",
            browser.name()
        );
        // A session that opens resets the count: the next not-ready open is a plain retry again.
        ready.store(1, Ordering::SeqCst);
        let open = gateway::call(&core.shared, "s", "chrome_session", args.clone())
            .await
            .unwrap();
        assert_eq!(
            open["vwMcpResult"]["structuredContent"]["sessionId"], "ready",
            "{open}"
        );
        core.shutdown().await;
    }
}
