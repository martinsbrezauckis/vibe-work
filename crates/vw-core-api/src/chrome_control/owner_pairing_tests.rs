//! Signed pairing-notification shape tests for the owner. Uses fixed fingerprints; no helper or browser runs.
use super::pairing_result;
use serde_json::json;

const OURS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn waiting_and_paired_show_our_fingerprint() {
    let waiting =
        pairing_result(&json!({"profileId":"p","state":"waiting","fingerprint":OURS})).unwrap();
    assert_eq!(
        waiting["structuredContent"]["error"]["code"],
        "OWNER_APPROVAL_REQUIRED"
    );
    assert_eq!(waiting["structuredContent"]["ownerFingerprint"], OURS);
    let paired =
        pairing_result(&json!({"profileId":"p","state":"paired","fingerprint":OURS})).unwrap();
    assert_eq!(
        paired["structuredContent"]["error"]["code"],
        "OWNER_CONNECTING"
    );
    assert_eq!(paired["structuredContent"]["ownerFingerprint"], OURS);
}

#[test]
fn mismatch_says_pair_again_and_never_takes_the_other_fingerprint() {
    let mismatch =
        pairing_result(&json!({"profileId":"p","state":"mismatch","fingerprint":OTHER})).unwrap();
    assert_eq!(
        mismatch["structuredContent"]["error"]["code"],
        "OWNER_IDENTITY_MISMATCH"
    );
    assert!(
        mismatch["structuredContent"]
            .get("ownerFingerprint")
            .is_none()
    );
}

#[test]
fn unknown_or_missing_state_is_ignored() {
    assert!(pairing_result(&json!({"state":"admit"})).is_none());
    assert!(pairing_result(&json!({})).is_none());
}

#[tokio::test(start_paused = true)]
async fn paired_notification_refreshes_extension_metadata_before_the_next_poll() {
    let core = crate::offline_core::start_with("pair-event-refresh", |config, _| {
        config.chrome_control.macos = Some(true);
    })
    .await;
    let shared = &core.shared;
    let calls = shared.chrome_control.owner.answering(|tool, _| {
        assert_eq!(tool, "chrome_status");
        json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,
            "extensionVersion":"0.1.6"}]}})
    });
    let mut events = core.subscribe(None);
    super::observe(&std::sync::Arc::downgrade(shared), &json!({
        "method":"notifications/owner/pairing", "params":{"profileId":"chrome:p","state":"paired","fingerprint":OURS}
    })).unwrap();
    // A refreshed event must arrive before the two-second periodic poll, with paused virtual time.
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while let Some(event) = events.next().await {
            if matches!(event, crate::CoreEvent::ChromeControl { view }
                if view.setup.addon.as_ref().and_then(|addon| addon.extension_version.as_deref()) == Some("0.1.6")) {
                return;
            }
        }
        panic!("Settings event stream ended");
    }).await.expect("Settings must receive the refreshed add-on view before the next poll");
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "paired must request status immediately"
    );
    assert_eq!(
        super::super::installation::version(shared, super::super::agent_browser::Browser::Chrome)
            .as_deref(),
        Some("0.1.6")
    );
    core.shutdown().await;
}
