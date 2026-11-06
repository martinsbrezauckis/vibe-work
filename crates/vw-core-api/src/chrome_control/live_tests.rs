//! The live connection decides Chrome calls; the card only words a refusal while nothing usable is connected.
//! Through the real gateway with the connected fixture (`test_support`); no browser.
use super::{AddonState, owner, test_support};
use serde_json::{Value, json};
use std::sync::Arc;

const SUPPORTED: &[&str] = &["clipboard.read", "page.read", "page.click"];

fn status() -> Value {
    json!({"structuredContent":{"sessions":[{"sessionId":"t","profileId":"chrome:p"}],
        "profiles":[{"profileId":"chrome:p","connected":true,"extensionVersion":"0.1.6",
            "features":SUPPORTED,"extensionCapabilities":SUPPORTED}]}})
}

async fn connected(name: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
    })
    .await;
    let shared = &core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("t".into(), "s".into());
    core
}

/// PREDICT (before the change): the helper's failure of one action (a closed tab) set the pairing record to "Not
/// connected" (`owner::update_pairing` read any error code as the connection's), so the card flipped and the gateway
/// answered with the card's "not connected" diagnosis instead of the action's own error. After it the card stays
/// Connected and the agent reads the helper's code.
#[tokio::test]
async fn an_action_failure_keeps_the_card_connected_and_its_own_error() {
    let core = connected("chrome-live-action-failure").await;
    let shared = &core.shared;
    let weak = Arc::downgrade(shared);
    let _seen = test_support::answering(shared, move |tool, _| {
        let reply = if tool == "chrome_status" {
            status()
        } else {
            json!({"structuredContent":{"status":"failed","error":{"code":"TARGET_CLOSED","message":"closed"}}})
        };
        // As the real owner does with every helper reply.
        owner::update_pairing(&weak, &reply);
        reply
    });
    assert_eq!(super::commands::addon(shared).state, AddonState::Connected);
    let answer = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.click","args":{"ref":"r"}}),
    )
    .await
    .unwrap();
    let body = &answer["vwMcpResult"]["structuredContent"];
    assert_eq!(
        body["untrustedPageContent"]["error"]["code"], "TARGET_CLOSED",
        "{answer}"
    );
    assert!(body.get("addon").is_none(), "{answer}");
    assert_eq!(super::commands::addon(shared).state, AddonState::Connected);
    core.shutdown().await;
}

/// PREDICT (before the change): a card that lags the live connection (here still Connecting while the helper's own
/// status for this call reports the chat's profile connected and supporting the action) refused working Chrome with
/// the card's text. After it the call reaches Chrome; without a live connected profile the card's words still answer.
#[tokio::test]
async fn a_live_connected_profile_decides_while_the_card_lags() {
    let core = connected("chrome-live-card-lags").await;
    let shared = &core.shared;
    let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let source = live.clone();
    let seen = test_support::answering(shared, move |tool, _| {
        if tool == "chrome_status" {
            let mut reply = status();
            reply["structuredContent"]["profiles"][0]["connected"] =
                json!(source.load(std::sync::atomic::Ordering::SeqCst));
            reply
        } else {
            json!({"structuredContent":{"status":"completed","data":{"text":"fixture"}}})
        }
    });
    shared.chrome_control.owner.pairing.lock().unwrap().0 =
        crate::chrome_control::PairingState::Connecting;
    assert_eq!(super::commands::addon(shared).state, AddonState::Connecting);
    let args = json!({"sessionId":"t","action":"clipboard.read","args":{}});
    let answer = super::gateway::call(shared, "s", "chrome_execute", args.clone())
        .await
        .unwrap();
    assert!(
        answer["vwMcpResult"]["structuredContent"]
            .get("addon")
            .is_none(),
        "{answer}"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(name, sent)| name == "chrome_execute" && sent["action"] == "clipboard.read")
    );
    live.store(false, std::sync::atomic::Ordering::SeqCst);
    let before = seen.lock().unwrap().len();
    let answer = super::gateway::call(shared, "s", "chrome_execute", args)
        .await
        .unwrap();
    let text = answer["vwMcpResult"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("addon_suggest"), "{text}");
    assert!(
        seen.lock().unwrap()[before..]
            .iter()
            .all(|(name, _)| name == "chrome_status")
    );
    core.shutdown().await;
}

/// PREDICT (before the change): while the card says Connecting (the pairing was just confirmed), every call was
/// refused with the card's words before Chrome was asked. After it the helper's answer decides: a call it runs
/// returns its result, and a call it cannot run yet gets the card's diagnosis and step.
#[tokio::test]
async fn while_connecting_the_helper_decides_and_the_card_words_its_refusal() {
    let core = connected("chrome-live-connecting").await;
    let shared = &core.shared;
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let source = ready.clone();
    let seen = test_support::answering(shared, move |tool, _| {
        if tool == "chrome_status" {
            status()
        } else if source.load(std::sync::atomic::Ordering::SeqCst) {
            json!({"structuredContent":{"status":"completed","data":{"text":"fixture text"}}})
        } else {
            json!({"structuredContent":{"status":"failed","error":{"code":"ADAPTER_UNAVAILABLE","message":"not yet"}}})
        }
    });
    shared.chrome_control.owner.pairing.lock().unwrap().0 =
        crate::chrome_control::PairingState::Connecting;
    assert_eq!(super::commands::addon(shared).state, AddonState::Connecting);
    let args = json!({"sessionId":"t","action":"page.read","args":{}});
    let answer = super::gateway::call(shared, "s", "chrome_execute", args.clone())
        .await
        .unwrap();
    assert!(
        answer["vwMcpResult"]["structuredContent"]
            .get("addon")
            .is_none(),
        "{answer}"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "chrome_execute")
    );
    ready.store(false, std::sync::atomic::Ordering::SeqCst);
    let answer = super::gateway::call(shared, "s", "chrome_execute", args)
        .await
        .unwrap();
    assert_eq!(
        answer["vwMcpResult"]["structuredContent"]["addon"]["stateCode"],
        json!(AddonState::Connecting),
        "{answer}"
    );
    core.shutdown().await;
}

/// PREDICT (before the change): when the helper connection ends during a call, the owner answers "Outcome unknown"
/// (`TRANSPORT_LOST`) and clears the pairing record; the gateway then replaced that answer with the card's "not
/// connected" diagnosis, so an action that may have run read as not run (an agent could repeat a form submission).
/// After it the unknown outcome stays the answer.
#[tokio::test]
async fn a_lost_connection_keeps_its_unknown_outcome() {
    let core = connected("chrome-live-transport-lost").await;
    let shared = &core.shared;
    let weak = Arc::downgrade(shared);
    let _seen = test_support::answering(shared, move |tool, _| {
        if tool == "chrome_status" {
            return status();
        }
        // As the owner does when the helper's pipe closes: the connection is cleared and the call has no reply.
        weak.upgrade()
            .unwrap()
            .chrome_control
            .owner
            .pairing
            .lock()
            .unwrap()
            .0 = crate::chrome_control::PairingState::Disconnected;
        panic!("the helper's pipe closed during the call");
    });
    let answer = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.click","args":{"ref":"r"}}),
    )
    .await
    .unwrap();
    let body = &answer["vwMcpResult"]["structuredContent"];
    assert_eq!(body["error"]["code"], "TRANSPORT_LOST", "{answer}");
    assert!(body.get("addon").is_none(), "{answer}");
    core.shutdown().await;
}
