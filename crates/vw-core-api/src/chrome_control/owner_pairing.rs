//! Routes trusted helper pairing events to each browser's pending Connect and enrollment state.
//! Depends on the owner's pairing view and enrollment; called by the owner's pipe observer.
use std::sync::{Arc, PoisonError, atomic::Ordering};

use serde_json::Value;

use super::super::{enroll, enroll_browser::Browser};
use crate::core::Shared;

pub(super) fn observe(shared: &Arc<Shared>, params: &Value, mut result: Value) {
    let browser = Browser::from_report(params["profileId"].as_str());
    let state = params["state"].as_str();
    if state == Some("paired")
        && let Some(profile) = params["profileId"].as_str()
    {
        shared.chrome_control.owner.approvals.paired(profile);
    }
    tracing::info!(
        state = enroll::state_word(state),
        ?browser,
        for_chrome = browser == Some(Browser::Chrome),
        connect_requested = Browser::Chrome.requested(shared).load(Ordering::SeqCst),
        edge_connect_requested = Browser::Edge.requested(shared).load(Ordering::SeqCst),
        "Chrome enrollment owner_observe"
    );
    if let Some(browser) = browser {
        browser
            .enroll(shared)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .observe_state(state, params, browser);
        if browser == Browser::Chrome && state == Some("mismatch") {
            shared.chrome_control.forget_paired();
        }
    }
    // Keep browser labels even in a Chrome-only app talking to a helper built with other browsers enabled.
    if params.get("profileId").is_some() {
        result["structuredContent"]["profileId"] = params["profileId"].clone();
    }
    super::update_pairing(&Arc::downgrade(shared), &result);
    if let Some(browser) = browser
        && matches!(state, Some("waiting" | "mismatch"))
        && browser.requested(shared).swap(false, Ordering::SeqCst)
    {
        enroll::on_waiting(shared, state == Some("waiting"), browser);
    }
}
