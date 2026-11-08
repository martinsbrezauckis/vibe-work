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
