//! Separates trusted helper pairing observations by browser; used by the opt-in owner supervisor.
use super::PairingState;
use crate::core::Shared;
use serde_json::Value;
use std::sync::{Arc, PoisonError};

/// Remembers a live Edge connection for display and reopening; an owner conflict or new pairing request revokes it.
pub(super) fn remember_pairing(chrome: &super::ChromeControl, paired: bool) {
    if paired {
        super::installation::remember_directory(chrome, super::agent_browser::Browser::Edge);
    } else {
        super::installation::forget_directory(chrome, super::agent_browser::Browser::Edge);
    }
    if chrome
        .edge_paired_once
        .swap(paired, std::sync::atomic::Ordering::SeqCst)
        != paired
        && let Some(dir) = chrome.dir()
        && let Err(reason) = super::store::save_paired_once(&dir.join("edge"), paired)
    {
        tracing::debug!(%reason, "the Edge connection history could not be saved");
    }
}

/// Chrome's scoped profile or the UUID reported by the shipped Chrome-only helper.
pub(super) fn chrome_profile(id: &str) -> bool {
    id.starts_with("chrome:") || super::enroll::uuid_shape(id)
}

fn state(data: &Value, browser: &str) -> Option<PairingState> {
    if browser == "firefox:"
        && let Some(state) = data["firefoxWindow"].as_str()
    {
        return PairingState::firefox_window(state);
    }
    if browser == "firefox:"
        && data["profileId"]
            .as_str()
            .is_some_and(|id| id != "firefox:agent")
    {
        return None;
    }
    if let Some(profile) = data["profileId"].as_str()
        && !profile.starts_with(browser)
    {
        return None;
    }
    let code = data["error"]["code"].as_str().unwrap_or_default();
    let event = data["profileId"].is_string();
    let connected = data["profiles"].as_array().is_some_and(|profiles| {
        profiles.iter().any(|p| {
            p["connected"] == true
                && (event
                    || p["profileId"].as_str().is_some_and(|p| {
                        if browser == "firefox:" {
                            p == "firefox:agent"
                        } else {
                            p.starts_with(browser)
                        }
                    }))
        })
    });
    // Action failures describe one operation, not the helper's connection state.
    if data.get("profiles").is_none() && !code.starts_with("OWNER_") {
        return None;
    }
    if browser == "firefox:" {
        return Some(if connected {
            PairingState::WindowOpen
        } else {
            PairingState::WindowClosed
        });
    }
    Some(PairingState::observe(
        code,
        connected,
        code == "OWNER_APPROVAL_REQUIRED",
    ))
}

pub(super) fn update(shared: &Arc<Shared>, data: &Value) {
    let owner = &shared.chrome_control.owner;
    let mut changed = super::firefox_sharing::observe(shared, data);
    for (browser, pairing) in [
        ("edge:", &owner.edge_pairing),
        ("firefox:", &owner.firefox_pairing),
    ] {
        let Some(state) = state(data, browser) else {
            continue;
        };
        let mut current = pairing.lock().unwrap_or_else(PoisonError::into_inner);
        if (browser == "edge:"
            && super::remove::is_removed(shared, super::agent_browser::Browser::Edge))
            || (browser == "firefox:"
                && super::remove::is_removed(shared, super::agent_browser::Browser::Firefox))
        {
            continue;
        }
        // No registered profile is expected while an enabled extension holds another identity.
        if current.0 == PairingState::OtherOwner
            && data.get("error").is_none()
            && state != PairingState::Paired
        {
            continue;
        }
        // A signed pairing event precedes adapter registration. Empty status polls must not erase it.
        let prefix = browser;
        if browser != "firefox:"
            && current.0 != PairingState::Unknown
            && (browser != "edge:" || current.0 != PairingState::Paired)
            && data["profiles"].as_array().is_some_and(|profiles| {
                !profiles.iter().any(|profile| {
                    profile["profileId"]
                        .as_str()
                        .is_some_and(|id| id.starts_with(prefix))
                })
            })
            && data.get("profileId").is_none()
            && data.get("error").is_none()
        {
            continue;
        }
        let fingerprint = data["ownerFingerprint"]
            .as_str()
            .map(|fp| fp.chars().take(16).collect())
            .or_else(|| current.1.clone());
        let next = (state, fingerprint);
        changed |= *current != next;
        *current = next;
        if browser == "edge:" && cfg!(feature = "your-edge") {
            if state == PairingState::Paired {
                remember_pairing(&shared.chrome_control, true);
            } else if matches!(state, PairingState::OtherOwner | PairingState::Waiting) {
                remember_pairing(&shared.chrome_control, false);
            }
        }
    }
    super::owner::publish_setup_if_changed(shared, changed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn action_refusals_do_not_change_browser_connection_state() {
        for browser in ["chrome:", "edge:", "firefox:"] {
            for code in ["OUTCOME_UNKNOWN", "UNSUPPORTED_ACTION", "INVALID_ARGS"] {
                let reply = json!({"profileId":format!("{browser}agent"),"error":{"code":code}});
                assert_eq!(state(&reply, browser), None, "{browser} {code}");
            }
        }
    }
    #[test]
    fn chrome_connecting_survives_browser_scoping() {
        assert_eq!(
            state(
                &json!({"profileId":"chrome:one", "error":{"code":"OWNER_CONNECTING"}}),
                "chrome:"
            ),
            Some(PairingState::Connecting)
        );
    }
    #[test]
    fn an_edge_connection_never_pairs_chrome() {
        let data = json!({"profiles":[{"profileId":"edge:one", "connected":true}]});
        assert_eq!(state(&data, "edge:"), Some(PairingState::Paired));
        assert_eq!(state(&data, "chrome:"), Some(PairingState::Disconnected));
        let event = json!({"profileId":"edge:one", "profiles":[{"connected":true}]});
        assert_eq!(state(&event, "chrome:"), None);
        assert_eq!(state(&event, "edge:"), Some(PairingState::Paired));
    }
    #[test]
    fn unscoped_pairing_events_are_not_attributed_to_chrome() {
        assert_eq!(
            state(
                &json!({"profileId":"forged", "error":{"code":"OWNER_APPROVAL_REQUIRED"}}),
                "chrome:"
            ),
            None
        );
    }
    #[tokio::test]
    async fn status_poll_keeps_signed_pairing_until_the_profile_registers() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let shared = &fixture.core.shared;
        update(
            shared,
            &json!({"profileId":"edge:one", "error":{"code":"OWNER_APPROVAL_REQUIRED"}}),
        );
        update(
            shared,
            &json!({"profiles":[], "firefoxWindow":"Window closed"}),
        );
        assert_eq!(
            shared.chrome_control.owner.edge_pairing.lock().unwrap().0,
            PairingState::Waiting
        );
        update(
            shared,
            &json!({"profiles":[{"profileId":"edge:one","connected":true}]}),
        );
        assert_eq!(
            shared.chrome_control.owner.edge_pairing.lock().unwrap().0,
            PairingState::Paired
        );
        update(
            shared,
            &json!({"profiles":[{"profileId":"edge:one","connected":false}]}),
        );
        assert_eq!(
            shared.chrome_control.owner.edge_pairing.lock().unwrap().0,
            PairingState::Disconnected
        );
        fixture.finish().await;
    }
}

#[cfg(all(test, feature = "your-firefox"))]
mod firefox_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn a_firefox_connection_never_pairs_chrome_or_edge() {
        let data = json!({"profiles":[{"profileId":"firefox:same","connected":true}]});
        assert_eq!(state(&data, "firefox:"), Some(PairingState::WindowClosed));
        assert_eq!(state(&data, "chrome:"), Some(PairingState::Disconnected));
        assert_eq!(state(&data, "edge:"), Some(PairingState::Disconnected));
    }
    #[tokio::test]
    async fn owned_window_state_has_no_addon_grants() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        update(
            &fixture.core.shared,
            &json!({"firefoxWindow":"Window open","profiles":[{"profileId":"firefox:agent","connected":true}]}),
        );
        let view = super::super::commands::view(&fixture.core.shared);
        let firefox = view.firefox.unwrap();
        assert_eq!(firefox.pairing, "Window open");
        assert!(firefox.states.is_none());
        assert!(firefox.browser_issue.is_none());
        assert!(firefox.install_state.is_none());
        fixture.finish().await;
    }
}
