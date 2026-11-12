//! Helper notification and status observations owned by the browser connection.
//! Called by the owner pipe loop and status readers; uses the existing pairing/work state and UI events.
//! Observations never grant authority or replay requests.
use super::super::PairingState;
use super::pairing;
use crate::core::Shared;
use serde_json::{Value, json};
#[cfg(test)]
use std::sync::Arc;
use std::sync::{PoisonError, Weak};

pub(in crate::chrome_control) fn publish(shared: &Shared) {
    shared.emit(crate::CoreEvent::ChromeControl {
        view: super::super::commands::view(shared),
    });
}

pub(in crate::chrome_control) fn observe(shared: &Weak<Shared>, message: &Value) -> Result<(), ()> {
    let Some(shared) = shared.upgrade() else {
        return Err(());
    };
    #[cfg(feature = "your-safari")]
    if message["method"] == "notifications/owner/safariAction" {
        super::super::safari_service::observe(&shared, &message["params"]);
    }
    if message["method"] == "notifications/owner/safari"
        && let Some(state) = message["params"]["state"].as_str()
    {
        super::super::safari::connection(&shared, state);
    }
    if message["method"] == "notifications/owner/pairingApproval" {
        super::super::pairing_approval::observe(&shared, &message["params"]);
    }
    if message["method"] == "notifications/client/selection" {
        let changed = shared
            .chrome_control
            .selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .receive(&message["params"], |id| shared.store.view(id).is_some());
        if changed {
            publish(&shared);
        }
    }
    if message["method"] == "notifications/client/share" {
        let changed = shared
            .chrome_control
            .selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .receive_share(&message["params"]);
        if changed {
            publish(&shared);
        }
    }
    if message["method"] == "notifications/owner/pairing"
        && let Some(result) = pairing_result(&message["params"])
    {
        super::super::safari::observe(&shared, &message["params"]);
        pairing::observe(&shared, &message["params"], result);
    }
    Ok(())
}

/// The helper's signed pairing state (protocol v6 `notifications/owner/pairing`) in the shape [`update_pairing`]
/// already reads. `waiting` / `paired` carry this Vibe Work's own fingerprint, shown so the person can compare; for
/// `mismatch` the fingerprint is the other identity Chrome holds, so it never replaces ours on screen. An unknown
/// state is ignored (the helper validates and bounds these events; this only informs, it admits nothing).
pub(super) fn pairing_result(params: &Value) -> Option<Value> {
    let fingerprint = params["fingerprint"].clone();
    match params["state"].as_str()? {
        "waiting" => Some(json!({"structuredContent":{
            "error":{"code":"OWNER_APPROVAL_REQUIRED"},"ownerFingerprint":fingerprint}})),
        "paired" => Some(json!({"structuredContent":{
            "error":{"code":"OWNER_CONNECTING"},"ownerFingerprint":fingerprint}})),
        "mismatch" => {
            Some(json!({"structuredContent":{"error":{"code":"OWNER_IDENTITY_MISMATCH"}}}))
        }
        _ => None,
    }
}

pub(in crate::chrome_control) fn update_pairing(shared: &Weak<Shared>, result: &Value) {
    update_pairing_state(shared, result);
    if let Some(shared) = shared.upgrade() {
        shared.chrome_control.owner.starts.connected(&shared);
        super::super::connection_notice::connected(&shared);
    }
}

fn update_pairing_state(shared: &Weak<Shared>, result: &Value) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let mut data = result["structuredContent"].clone();
    if !super::super::remove::filter_observation(&shared, &mut data) {
        return;
    }
    super::super::profile_bindings::observe(&shared.chrome_control, &data);
    super::super::profile_presence::reconcile(&shared);
    shared.chrome_control.owner.wake.observe(&data);
    shared.chrome_control.owner.profile_ids.observe(&data);
    // Keep all browsers' versions before the Chrome-only pairing view below filters them.
    if data["profiles"].is_array() && data.get("profileId").is_none() {
        shared
            .chrome_control
            .owner
            .connected_profiles
            .observe(&data);
    }
    if cfg!(any(feature = "your-edge", feature = "your-firefox")) {
        super::super::edge_state::update(&shared, &data);
    }
    if data["profileId"]
        .as_str()
        .is_some_and(|id| !super::super::edge_state::chrome_profile(id))
    {
        return;
    }
    if let Some(profiles) = data.get_mut("profiles").and_then(Value::as_array_mut) {
        profiles.retain(|profile| {
            let id = profile["profileId"].as_str();
            if cfg!(any(feature = "your-edge", feature = "your-firefox")) {
                id.is_some_and(super::super::edge_state::chrome_profile)
            } else {
                // Chrome-only helpers historically used unscoped labels; foreign scopes remain foreign.
                id.is_none_or(|id| !id.contains(':') || id.starts_with("chrome:"))
            }
        });
    }
    let data = &data;
    let code = data["error"]["code"].as_str().unwrap_or_default();
    let connected = data["profiles"]
        .as_array()
        .is_some_and(|p| p.iter().any(|p| p["connected"] == true));
    // Only a status answer (its profiles) or the helper's own pairing/owner report says what is connected. An
    // action's failure (a closed tab, an unknown profile, an older extension) says nothing about the connection:
    // taking it as "Not connected" flipped the card for two seconds and replaced the action's own error.
    if data.get("profiles").is_none() && !code.starts_with("OWNER_") {
        return;
    }
    // Every helper status answer names our own fingerprint, with or without a waiting extension: only the helper's
    // waiting report is the pairing step (`addon`), else nothing is connected.
    let waiting = shared
        .chrome_control
        .owner
        .enroll
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .waiting_reported();
    let state = PairingState::observe(code, connected, waiting);
    // A status poll has no registered profile while an enabled extension holds another identity.
    // Keep its signed mismatch until a pairing report, live connection or helper reset replaces it.
    if !connected
        && code.is_empty()
        && shared
            .chrome_control
            .owner
            .pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            == PairingState::OtherOwner
    {
        return;
    }
    apply_pairing_state(&shared, data, state, connected);
}

fn apply_pairing_state(shared: &Shared, data: &Value, state: PairingState, connected: bool) {
    let mut current = shared
        .chrome_control
        .owner
        .pairing
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    // Removal clears this same lock after deleting the registration. A shared status
    // poll may still name the owner's fingerprint, even with no Chrome profiles left.
    if super::super::remove::is_removed(shared, super::super::agent_browser::Browser::Chrome) {
        return;
    }
    if connected {
        shared.chrome_control.mark_paired();
        shared
            .chrome_control
            .owner
            .enroll
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .connected();
    }
    let fingerprint = data["ownerFingerprint"]
        .as_str()
        .map(|fingerprint| fingerprint.chars().take(16).collect())
        .or_else(|| current.1.clone());
    let next = (state, fingerprint);
    let changed = *current != next;
    *current = next;
    drop(current);
    publish_setup_if_changed(shared, changed);
}

/// A regular status poll also discovers either browser's extension directory appearing after pairing.
pub(in crate::chrome_control) fn publish_setup_if_changed(shared: &Shared, changed: bool) {
    let view = super::super::commands::view(shared);
    let next = view.clone();
    let mut previous = shared
        .chrome_control
        .owner
        .observed_setup
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if changed || previous.as_ref() != Some(&next) {
        *previous = Some(next);
        drop(previous);
        shared.emit(crate::CoreEvent::ChromeControl { view });
    }
}

#[cfg(test)]
pub(in crate::chrome_control) fn update_work_for_test(shared: &Arc<Shared>, result: &Value) {
    update_work(&Arc::downgrade(shared), result, tokio::time::Instant::now());
}

pub(in crate::chrome_control) fn update_work(
    shared: &Weak<Shared>,
    result: &Value,
    started: tokio::time::Instant,
) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let Some(rows) = result["structuredContent"]["sessions"].as_array() else {
        return;
    };
    let mut tasks = shared
        .chrome_control
        .tasks
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut work = shared
        .chrome_control
        .work
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    // A live session the helper reports stopped without Vibe Work asking: the person's Stop in Chrome.
    let stopped = work.stopped_unasked(rows, started);
    let changed = work.observe(rows, &mut tasks, started);
    drop(work);
    drop(tasks);
    super::super::profile_bindings::acknowledge(&shared.chrome_control, rows);
    super::super::person_stop::received(&shared, &stopped);
    if changed {
        publish(&shared);
    }
}
