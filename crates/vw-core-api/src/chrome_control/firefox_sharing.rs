//! Firefox add-on sharing status, separate from the owned automation window. Consumed by Settings and agent facts.
use super::{AddonState, AddonView, agent_browser::Browser};
use crate::core::Shared;
use crate::{CommandResult, Core, CoreError, Origin};
use serde_json::json;
use vw_platform::sync::LockRecover;

/// Firefox page sharing ("Ask Vibe Work" through the signed add-on). The helper relays the add-on and limits it to a
/// person's shares (no agent session on the person's Firefox). When false: Settings and agent facts carry no sharing
/// view, Firefox setup registers the helper without opening the add-on prompt, and the add-on request is refused.
/// The owned Firefox window (`WebDriver` `BiDi`, opened by itself when a chat browses) is separate and unaffected.
pub(super) const PAGE_SHARING_AVAILABLE: bool = true;

/// What the agent is told about a connected sharing add-on: shares arrive from the person; it cannot reach the tabs.
const CONNECTED: &str = "The user can share pages, links, images and selected text from their own Firefox with Ask Vibe Work in Firefox's right-click menu; shared items arrive in the chat. The agent cannot open, list or read the user's Firefox tabs. For browser actions use chrome_session with profileId firefox:agent (the owned Firefox window).";
const TOOLBAR_WAKE: &str = "If the add-on is already installed, open it once from Firefox's toolbar (the Vibe Work icon) to connect.";

/// The one step that changes an already paired Vibe Work: it stays a choice inside the Firefox extension, never a
/// Settings action. The extension's name comes from the shared vocabulary. The toolbar is named because the add-on
/// accepts Pair only from its toolbar panel; its page opened as a tab from Firefox Add-ons is refused.
fn change_in_extension() -> String {
    format!(
        "open {} from the Firefox toolbar and choose Pair with Vibe Work to change it.",
        vw_vocabulary::label("workNames", "chromeExtension")
    )
}

/// `change_in_extension` as a sentence of its own.
fn change_in_extension_sentence() -> String {
    let step = change_in_extension();
    format!("O{}", &step[1..])
}

/// The plain refusal for the add-on request while page sharing is hidden; says how the agent browses instead (the
/// owned window opens by itself when a chat browses; there is no Open window control since 0.1.17).
pub(super) fn page_sharing_unavailable() -> String {
    Browser::Firefox.words(
        "Sharing pages from {browser} is not available in this version of Vibe Work. When a chat uses {yourBrowser}, the agent opens its own {browser} window by itself.",
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
        && matches!(current.0, PairingState::Waiting | PairingState::OtherOwner)
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
    let fingerprint = if state == PairingState::Waiting {
        data["ownerFingerprint"]
            .as_str()
            .filter(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(|value| value[..16].to_owned())
            .or_else(|| {
                (current.0 == PairingState::Waiting)
                    .then(|| current.1.clone())
                    .flatten()
            })
    } else {
        None
    };
    let changed = *current != (state, fingerprint.clone());
    current.0 = state;
    current.1 = fingerprint;
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
        vw_vibe_install::support::compare_versions(v, "0.1.8") == Some(std::cmp::Ordering::Less)
    }) {
        AddonState::UpdateNeeded
    } else if !live.is_empty() {
        AddonState::Connected
    } else if pairing == PairingState::Waiting {
        AddonState::NotPaired
    } else if pairing == PairingState::Connecting {
        AddonState::Connecting
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
        view.label = "Waiting for you to pair".into();
        view.instruction = "The Firefox extension is ready. Ask the user to select Pair Firefox in the Firefox card in Vibe Work Settings > Browser.".into();
        view.action = None;
    }
    if state == AddonState::OtherOwner {
        view.label = "Identity changed".into();
        view.instruction = format!(
            "Firefox is paired with another Vibe Work. Ask the user to {}",
            change_in_extension()
        );
        view.action = None;
    }
    if matches!(
        state,
        AddonState::NotSetUp | AddonState::ExtensionNotInstalled | AddonState::NotConnected
    ) {
        view.label = "Not connected".into();
        view.instruction = "Ask the user to open Firefox. If the extension is missing, add it from the Firefox card in Settings > Browser. When it reaches Vibe Work, pair Firefox in Settings.".into();
        if state == AddonState::NotConnected
            && probe.status == super::firefox_probe::Status::Installed
        {
            // The add-on is installed, yet it has not reached Vibe Work: opening its toolbar panel starts its
            // background page (add-on 0.1.7 did not start with Firefox).
            view.instruction.push(' ');
            view.instruction.push_str(TOOLBAR_WAKE);
        }
    }
    if state == AddonState::UpdateNeeded {
        self_distributed_update(&mut view);
    }
    view
}

impl Core {
    /// Only a real Settings click may ask the owner pipe to offer its connection-bound ticket.
    pub(super) async fn firefox_pair(&self, origin: Origin) -> Result<CommandResult, CoreError> {
        if crate::command_origin::debug_api_acts(&self.shared, origin) {
            return Err(super::commands::invalid(
                "Pair Firefox in the Vibe Work Settings window.",
            ));
        }
        let state = view(&self.shared).state;
        if state == AddonState::OtherOwner {
            return Err(super::commands::invalid(&change_in_extension_sentence()));
        }
        if state != AddonState::NotPaired {
            return Err(super::commands::invalid(
                "Firefox is not waiting to pair. Check its state on the Firefox card in Settings > Browser.",
            ));
        }
        let offered = self
            .shared
            .chrome_control
            .owner
            .rpc("owner.firefoxPair", json!({}))
            .await;
        if !offered
            .as_ref()
            .is_some_and(|answer| answer["offered"] == true)
        {
            let changed = change_in_extension_sentence();
            let reason = match offered.as_ref().and_then(|answer| answer["none"].as_str()) {
                Some("ambiguous") => {
                    "More than one Firefox is waiting. Close the other Firefox profile and try again."
                }
                Some("identity_changed") => changed.as_str(),
                _ => "Firefox is no longer waiting. Open Firefox and try again.",
            };
            return Err(super::commands::invalid(reason));
        }
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
            hint: None,
        })
    }
}

/// The signed add-on is self-distributed (no listing, no automatic update): a newer one comes from Vibe Work's own
/// copy through Connect, not from Firefox updating it by itself as the shared extension wording says.
fn self_distributed_update(view: &mut AddonView) {
    let firefox = vw_vocabulary::text("firefoxName").unwrap_or_default();
    view.label = format!(
        "{firefox} add-on is older than this Vibe Work needs. Choose Connect to update it."
    );
    view.instruction = format!(
        "The add-on in {firefox} is older than this Vibe Work needs. Ask the user to choose Connect next to Ask Vibe Work in Settings > Browser > {firefox} to update it."
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
        assert_eq!(view(shared).label, "Waiting for you to pair");
        assert!(
            view(shared)
                .instruction
                .contains("Pair Firefox in the Firefox card in Vibe Work Settings")
        );
        super::super::edge_state::update(
            shared,
            &serde_json::json!({
                "profileId":"firefox:12345678-1234-1234-1234-123456789012", "error":{"code":"OWNER_CONNECTING"}
            }),
        );
        assert_eq!(view(shared).state, AddonState::Connecting);
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
    async fn waiting_firefox_identity_is_short_and_survives_an_empty_status_poll() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let shared = &fixture.core.shared;
        let id = "firefox:12345678-1234-1234-1234-123456789012";
        let fingerprint = "0123456789abcdef".repeat(4);
        assert!(observe(
            shared,
            &serde_json::json!({
                "profileId":id,"error":{"code":"OWNER_APPROVAL_REQUIRED"},"ownerFingerprint":fingerprint
            })
        ));
        assert_eq!(view(shared).state, AddonState::NotPaired);
        assert_eq!(
            super::super::firefox_commands::setup_view(shared)
                .unwrap()
                .fingerprint
                .as_deref(),
            Some("0123456789abcdef")
        );
        assert!(!observe(shared, &serde_json::json!({"profiles":[]})));
        assert_eq!(view(shared).state, AddonState::NotPaired);
        fixture.finish().await;
    }
    #[tokio::test]
    async fn changed_firefox_identity_stays_visible_until_the_connection_changes() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let shared = &fixture.core.shared;
        assert!(observe(
            shared,
            &serde_json::json!({
                "profileId":"firefox:12345678-1234-1234-1234-123456789012",
                "error":{"code":"OWNER_IDENTITY_MISMATCH"}
            })
        ));
        assert_eq!(view(shared).state, AddonState::OtherOwner);
        assert_eq!(
            view(shared).instruction,
            format!(
                "Firefox is paired with another Vibe Work. Ask the user to open {} from the Firefox toolbar and choose Pair with Vibe Work to change it.",
                vw_vocabulary::label("workNames", "chromeExtension")
            )
        );
        let error = fixture
            .core
            .firefox_pair(crate::Origin::Ui)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Pair with Vibe Work"), "{error}");
        assert!(!observe(shared, &serde_json::json!({"profiles":[]})));
        assert_eq!(view(shared).state, AddonState::OtherOwner);
        fixture.finish().await;
    }
    #[tokio::test]
    async fn a_debug_api_cannot_pair_firefox_even_through_the_window() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let direct = fixture
            .core
            .firefox_pair(crate::Origin::DebugApi)
            .await
            .unwrap_err();
        assert!(direct.to_string().contains("Settings window"), "{direct}");
        fixture.core.set_window_drive(|| true);
        let driven = fixture
            .core
            .firefox_pair(crate::Origin::Ui)
            .await
            .unwrap_err();
        assert!(driven.to_string().contains("Settings window"), "{driven}");
        fixture.finish().await;
    }
    #[tokio::test]
    async fn waiting_uses_only_the_fixtures_absent_old_or_current_addon() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let shared = &fixture.core.shared;
        let config = &shared.chrome_control.config;
        let home = config.user_home.as_ref().expect("isolated browser home");
        let root = if config.is_macos() {
            home.join("Library/Application Support/Firefox/Profiles")
        } else if cfg!(windows) {
            home.join("AppData/Roaming/Mozilla/Firefox/Profiles")
        } else {
            home.join(".mozilla/firefox")
        };
        let profile = root.join("test.default");
        std::fs::create_dir_all(&profile).unwrap();
        observe(
            shared,
            &json!({
                "profileId":"firefox:test", "error":{"code":"OWNER_APPROVAL_REQUIRED"}
            }),
        );
        for (version, expected) in [
            (None, AddonState::NotPaired),
            (Some("0.1.7"), AddonState::UpdateNeeded),
            (Some("0.1.8"), AddonState::NotPaired),
        ] {
            let addons = version.map_or_else(Vec::new, |version| {
                vec![json!({
                    "id":super::super::setup::FIREFOX_EXTENSION_ID,
                    "version":version, "userDisabled":false, "appDisabled":false
                })]
            });
            std::fs::write(
                profile.join("extensions.json"),
                json!({"addons":addons}).to_string(),
            )
            .unwrap();
            assert_eq!(view(shared).state, expected, "add-on {version:?}");
            assert_eq!(view(shared).extension_version.as_deref(), version);
        }
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
