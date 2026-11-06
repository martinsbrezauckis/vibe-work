//! Reconnection reaches only the refused active chat/turn, once, and still obeys a concurrent Stop.
use super::*;
use crate::chrome_control::{edge_agent_tests::fixture, gateway};

async fn waiting(name: &str) -> Core {
    let core = fixture(name, json!([])).await;
    core.shared.chrome_control.mark_paired();
    core.shared
        .chrome_control
        .owner
        .pairing
        .lock()
        .expect("pairing state")
        .0 = crate::chrome_control::PairingState::Connecting;
    core.shared.chrome_control.owner.answering(|_, _| {
        crate::chrome_control::owner::failure(
            "The native port is reconnecting",
            "PROFILE_NOT_CONNECTED",
        )
    });
    let _ = core.shared.store.load(json!({"session":{"id":"s","status":{"activeTurnId":"t1"}},"history":[],"turns":[{"id":"t1","status":"in_progress"}]}), vw_session::LoadKind::New).expect("load active chat");
    let result = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .expect("connection refusal");
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["addon"]["stateCode"], "connecting",
        "{result}"
    );
    core
}

fn paired(core: &Core, browser: Browser) {
    let owner = &core.shared.chrome_control.owner;
    let pairing = if browser == Browser::Chrome {
        &owner.pairing
    } else {
        &owner.edge_pairing
    };
    pairing.lock().expect("pairing state").0 = crate::chrome_control::PairingState::Paired;
}

#[tokio::test]
async fn reconnect_notifies_the_waiting_browser_once() {
    let core = waiting("browser-notice-once").await;
    assert!(take(&core.shared).is_empty());
    paired(&core, Browser::Edge);
    assert!(
        take(&core.shared).is_empty(),
        "another browser cannot wake this wait"
    );
    paired(&core, Browser::Chrome);
    let notices = take(&core.shared);
    assert_eq!(notices.len(), 1, "the active chat receives the reconnect");
    assert_eq!(notices[0].session, "s");
    assert_eq!(notices[0].turn, "t1");
    assert!(
        take(&core.shared).is_empty(),
        "ordinary status polls never repeat it"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn reconnect_does_not_wake_a_later_turn() {
    let core = waiting("browser-notice-next-turn").await;
    let _ = core.shared.store.load(json!({"session":{"id":"s","status":{"activeTurnId":"t2"}},"history":[],"turns":[{"id":"t2","status":"in_progress"}]}), vw_session::LoadKind::New).expect("load active chat");
    paired(&core, Browser::Chrome);
    assert!(take(&core.shared).is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn a_stop_after_reconnection_cancels_the_queued_notice() {
    let core = waiting("browser-notice-stop").await;
    paired(&core, Browser::Chrome);
    let notice = take(&core.shared).pop().expect("a reconnect notice");
    crate::stops::stopped(&core.shared, "s");
    let error = notice.send(core.shared.clone()).await.unwrap_err();
    assert!(crate::stops::refused_as_stopped(&error), "{error:?}");
    core.shutdown().await;
}

#[tokio::test]
async fn edge_reconnect_notifies_even_when_chrome_keeps_another_owner_diagnosis() {
    let core = waiting("browser-notice-edge-other-owner").await;
    let owner = &core.shared.chrome_control.owner;
    // This chat is waiting for Edge; Chrome's signed other-owner diagnosis must remain intact.
    owner
        .connection_waiting
        .0
        .lock()
        .unwrap()
        .get_mut("s")
        .unwrap()
        .browser = Browser::Edge;
    owner.pairing.lock().unwrap().0 = crate::chrome_control::PairingState::OtherOwner;
    crate::chrome_control::owner::update_pairing(
        &Arc::downgrade(&core.shared),
        &json!({"structuredContent":{"profiles":[{"profileId":"edge:p","connected":true,"extensionVersion":"0.1.5"}]}}),
    );
    assert!(
        owner.connection_waiting.0.lock().unwrap().is_empty(),
        "Edge's reconnect must deliver the pending notice"
    );
    assert_eq!(
        owner.pairing.lock().unwrap().0,
        crate::chrome_control::PairingState::OtherOwner
    );
    core.shutdown().await;
}
