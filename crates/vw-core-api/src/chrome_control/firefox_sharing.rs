//! Firefox add-on sharing status, separate from the owned automation window. Consumed by Settings and agent facts.
use super::{AddonState, AddonView, agent_browser::Browser};
use crate::core::Shared;
use vw_platform::sync::LockRecover;

/// Firefox page sharing ("Ask Vibe Work" through the signed add-on). The helper relays the add-on and limits it to a
/// person's shares (no agent session on the person's Firefox). When false: Settings and agent facts carry no sharing
/// view, Firefox setup registers the helper without opening the add-on prompt, and the add-on request is refused.
/// The owned Firefox window (`WebDriver` `BiDi`, Open window) is separate and unaffected.
pub(super) const PAGE_SHARING_AVAILABLE: bool = true;

/// What the agent is told about a connected sharing add-on: shares arrive from the person; it cannot reach the tabs.
const CONNECTED: &str = "The user can share pages, links, images and selected text from their own Firefox with Ask Vibe Work in Firefox's right-click menu; shared items arrive in the chat. The agent cannot open, list or read the user's Firefox tabs. For browser actions use chrome_session with profileId firefox:agent (the owned Firefox window).";

/// The plain refusal for the add-on request while page sharing is hidden; names the owned window's control instead.
pub(super) fn page_sharing_unavailable() -> String {
    Browser::Firefox.words(
        "Sharing pages from {browser} is not available in this version of Vibe Work. Use Open window in Settings > Browser > {yourBrowser} instead.",
    )
}

/// The sharing view for Settings and agent facts; `None` while page sharing is hidden (the UI then hides the row).
pub(super) fn shown(shared: &Shared) -> Option<AddonView> {
    PAGE_SHARING_AVAILABLE.then(|| view(shared))
}

/// The core's own half of the person-share rule (the helper refuses too): an agent tool call that names the
/// person's Firefox profile is refused before any helper call. A bare status read stays allowed; it only reports.
pub(super) fn refuse_agent_reach(tool: &str, args: &serde_json::Value) -> Result<(), String> {
    let named = args["profileId"].as_str().is_some_and(addon_profile);
    let bare_status =
        tool == "chrome_status" && args.get("action").is_none() && args.get("actions").is_none();
    if named && !bare_status {
        return Err(AGENT_REFUSAL.into());
    }
    Ok(())
}
const AGENT_REFUSAL: &str = "Not run: the user's own Firefox only sends pages they choose with Ask Vibe Work; the agent cannot open, list or read its tabs. Use chrome_session with profileId firefox:agent (the owned Firefox window) for browser actions.";

/// Only the add-on profile may supply sharing facts; the owned `BiDi` runtime is a different connection. The
/// person's Firefox reached this way is read for the composer only, never bound to a chat as its browser.
pub(super) fn addon_profile(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.starts_with("firefox:") && id != "firefox:agent"
}

pub(super) fn observe(shared: &Shared, data: &serde_json::Value) -> bool {
    use super::PairingState;
    if data["profileId"]
        .as_str()
        .is_some_and(|id| !addon_profile(id))
    {
        return false;
    }
    let profiles = data["profiles"].as_array();
    let code = data["error"]["code"].as_str().unwrap_or_default();
    if profiles.is_none() && code.is_empty() {
        return false;
    }
    let mut current = shared
        .chrome_control
        .owner
        .firefox_share_pairing
        .lock_or_recover();
    if data["profileId"].is_null()
        && profiles.is_some_and(|rows| {
            !rows
                .iter()
                .any(|p| p["profileId"].as_str().is_some_and(addon_profile))
        })
        && current.0 == PairingState::Waiting
    {
        return false;
    }
    let connected = profiles.is_some_and(|rows| {
        rows.iter().any(|p| {
            p["connected"] == true
                && (p["profileId"].as_str().is_some_and(addon_profile)
                    || data["profileId"].as_str().is_some_and(addon_profile))
        })
    });
    let state = PairingState::observe(code, connected, code == "OWNER_APPROVAL_REQUIRED");
    let changed = current.0 != state;
    current.0 = state;
    changed
}

pub(super) fn view(shared: &Shared) -> AddonView {
    use super::PairingState;
    let chrome = &shared.chrome_control;
    let pairing = chrome.owner.firefox_share_pairing.lock_or_recover().0;
    let live: Vec<_> = chrome
        .owner
        .connected_profiles
        .snapshot()
        .into_iter()
        .filter(|(id, _)| addon_profile(id))
        .collect();
    let probe = super::firefox_probe::read(&chrome.config);
    let version = live
        .iter()
        .filter_map(|(_, version)| version.clone())
        .min_by(|a, b| {
            vw_vibe_install::support::compare_versions(a, b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .or(probe.version);
    let state = if !chrome.firefox_enabled() {
        AddonState::SwitchedOff
    } else if version.as_deref().is_some_and(|v| {
        vw_vibe_install::support::compare_versions(v, "0.1.7") == Some(std::cmp::Ordering::Less)
    }) {
        AddonState::UpdateNeeded
    } else if !live.is_empty() {
        AddonState::Connected
    } else if pairing == PairingState::Waiting {
        AddonState::NotPaired
    } else if pairing == PairingState::OtherOwner {
        AddonState::OtherOwner
    } else if !super::setup::registered(&chrome.config, chrome.dir(), "Mozilla Firefox") {
        AddonState::NotSetUp
    } else if probe.status == super::firefox_probe::Status::Disabled {
        AddonState::TurnedOffInChrome
    } else if probe.status == super::firefox_probe::Status::Missing {
        AddonState::ExtensionNotInstalled
    } else {
        AddonState::NotConnected
    };
    let mut view = super::addon::view_for(state, "", Browser::Firefox);
    view.extension_version = version;
    if state == AddonState::Connected {
        view.instruction = CONNECTED.into();
    }
    if state == AddonState::NotPaired {
        view.instruction = "The Firefox add-on is waiting for approval. Ask the user to confirm the Firefox connection in Vibe Work.".into();
        view.action = None;
    }
    if state == AddonState::UpdateNeeded {
        self_distributed_update(&mut view);
    }
    view
}

/// The signed add-on is self-distributed (no listing, no automatic update): a newer one comes from Vibe Work's own
/// copy through Connect, not from Firefox updating it by itself as the shared extension wording says.
fn self_distributed_update(view: &mut AddonView) {
    let firefox = vw_vocabulary::text("firefoxName").unwrap_or_default();
    view.instruction = format!(
        "The Vibe Work add-on in Firefox is older than this Vibe Work needs, and Firefox does not update it by itself. Ask the user to click Connect next to Ask Vibe Work in Settings > Browser > {firefox} and add the newer add-on when Firefox asks."
    );
    view.action = Some(super::addon::AddonAction::Connect);
}

impl super::owner::Owner {
    pub(super) fn clear_firefox(&self) {
        use super::PairingState;
        *self.firefox_pairing.lock_or_recover() = (PairingState::Unknown, None);
        *self.firefox_share_pairing.lock_or_recover() = (PairingState::Unknown, None);
        self.firefox_states.lock_or_recover().clear();
    }
}

#[cfg(all(test, feature = "your-firefox"))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sharing_off_and_waiting_are_independent_of_the_owned_window() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let shared = &fixture.core.shared;
        shared
            .chrome_control
            .set_browser_enabled(Browser::Firefox, false)
            .unwrap();
        assert_eq!(view(shared).state, AddonState::SwitchedOff);
        shared
            .chrome_control
            .set_browser_enabled(Browser::Firefox, true)
            .unwrap();
        super::super::edge_state::update(
            shared,
            &serde_json::json!({"profileId":"firefox:12345678-1234-1234-1234-123456789012", "error":{"code":"OWNER_APPROVAL_REQUIRED"}}),
        );
        assert_eq!(view(shared).state, AddonState::NotPaired);
        assert_ne!(
            shared
                .chrome_control
                .owner
                .firefox_pairing
                .lock()
                .unwrap()
                .0,
            super::super::PairingState::WindowOpen
        );
        fixture.finish().await;
    }
    #[tokio::test]
    async fn outdated_live_addon_is_detected_with_multi_digit_versions() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        fixture
            .core
            .shared
            .chrome_control
            .owner
            .connected_profiles
            .observe(&serde_json::json!({"profiles":[
                {"profileId":"firefox:one","connected":true,"extensionVersion":"0.1.10"},
                {"profileId":"firefox:two","connected":true,"extensionVersion":"0.1.6"}
            ]}));
        assert_eq!(view(&fixture.core.shared).state, AddonState::UpdateNeeded);
        fixture.finish().await;
    }
}

#[cfg(test)]
mod update_tests {
    #[test]
    fn an_outdated_firefox_add_on_is_replaced_through_connect_not_by_firefox_itself() {
        use super::{AddonState, Browser};
        let mut view =
            super::super::addon::view_for(AddonState::UpdateNeeded, "", Browser::Firefox);
        assert!(
            view.instruction.contains("by itself within a few hours"),
            "shared wording changed"
        );
        super::self_distributed_update(&mut view);
        assert_eq!(view.action, Some(super::super::addon::AddonAction::Connect));
        assert!(view.instruction.contains(&format!(
            "Settings > Browser > {}",
            vw_vocabulary::text("firefoxName").unwrap()
        )));
        assert!(view.instruction.contains("Ask the user"));
        assert!(!view.instruction.contains("within a few hours"));
    }
}
