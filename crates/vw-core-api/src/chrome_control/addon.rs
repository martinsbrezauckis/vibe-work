//! The one state model of Your Chrome, shared by the Connect Chrome card and the agent (`session_status`,
//! `addon_suggest`). Each state has the card's short label, the card's one action (or none) and a one-line
//! instruction for the person, which the agent reads with its reason.
//!
//! Invariants:
//! - Connected comes only from a live registered profile; files (extension folder, preferences) can only lower a
//!   label, never raise it to Connected.
//! - The pairing step (Pair in Chrome, the compare code) shows only after the helper reported a waiting extension;
//!   a helper status answer alone (it always names our own fingerprint) is not a waiting extension.
//! - A profile paired earlier and not connected now (Chrome closed) never shows the pairing step again.
//! - Chrome's preferences are read only when no live state decided first (`probe`).
//!
//! Depends on: `browser` (issues), `probe` (Chrome's files). Called by: `commands::view`, `agent_tools`.
use serde::{Deserialize, Serialize};

use super::browser::BrowserIssue;
use super::probe::Probe;
#[path = "addon_view.rs"]
mod view_impl;
pub(super) use view_impl::{
    reconnecting, reconnecting_for, window_closed_for, window_or_approval_for,
};
pub(crate) use view_impl::{view, view_for};

/// The instruction while another Vibe Work browser helper holds the connection. The owner keeps retrying, so
/// ending that helper is enough; Connect cannot help while it runs.
pub(super) const OTHER_HELPER_RUNNING: &str = "Another Vibe Work browser helper is still running and holds the connection to {browser}. Ask the user to quit any other Vibe Work copy; if none is open, end the chrome-control process (Activity Monitor or Task Manager) or restart the computer. It then connects by itself.";

/// Where Your Chrome stands, most blocking first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "ChromeAddonState"))]
pub enum AddonState {
    /// This build has no Chrome connection.
    Unavailable,
    /// Connect Chrome was never run.
    NotSetUp,
    /// No Google Chrome was found.
    ChromeMissing,
    /// Only a Snap or Flatpak Chrome was found.
    ChromeSandboxed,
    /// Only another Chromium browser was found.
    OtherBrowser,
    /// Your Chrome is switched off in Vibe Work.
    SwitchedOff,
    /// The extension's store listing is in Google's review.
    InReview,
    /// Chrome is paired with another Vibe Work.
    OtherOwner,
    /// Pairing is under way.
    Connecting,
    /// The extension waits to be paired with this Vibe Work.
    NotPaired,
    /// Another Vibe Work browser helper still holds the connection, so this app's helper cannot start.
    OtherHelperRunning,
    /// Vibe Work's Chrome helper is not running.
    HelperNotRunning,
    /// The device credential was deleted outside the app; remove the old registration before connecting.
    OwnerCredentialMissing,
    /// This computer has no usable system keyring, so the device credential cannot be read and the helper cannot start.
    OwnerKeyringUnavailable,
    /// The recorded native registration could not be repaired.
    HelperSetupRequired,
    /// macOS refused filesystem access during this browser's setup.
    AccessBlocked,
    /// Chrome turned the extension off (for example after an update that asked for new permissions).
    TurnedOffInChrome,
    /// Chrome must restart to find the extension (macOS).
    RestartChrome,
    /// The installed extension is older than this Vibe Work needs.
    UpdateNeeded,
    /// Paired earlier; Chrome is not open now.
    ChromeClosed,
    /// Chrome asks the person to enable the newly added extension.
    EnableInChrome,
    /// The extension is not in Chrome.
    ExtensionNotInstalled,
    /// Nothing is connected and nothing more is known.
    NotConnected,
    /// A live registered profile.
    Connected,
}

/// The card's one action for a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "ChromeAddonAction"))]
pub enum AddonAction {
    /// Connect Chrome (set up, add the extension, start the helper).
    Connect,
    /// Retry the owned helper registration.
    SetupAgain,
    /// Remove the old registration before creating a replacement identity.
    Remove,
    /// Bring the extension's pairing page forward.
    OpenPairingPage,
    /// Turn Your Chrome on in Vibe Work.
    TurnOn,
}

/// What the card shows and the agent reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "ChromeAddonView"))]
pub struct AddonView {
    /// Where it stands.
    pub state: AddonState,
    /// This browser's installed extension version, including while off; null when not installed.
    /// Older saved views may omit it; current core views always serialize it.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub extension_version: Option<String>,
    /// The card's short label.
    pub label: String,
    /// The card's one action; `None`: the next step is in Chrome or needs nothing.
    pub action: Option<AddonAction>,
    /// One line for the person: why, and what to do.
    pub instruction: String,
    /// A short instruction for a step in Chrome; absent when no such step is needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub step: Option<String>,
}

/// What is known now, from the live connection, Vibe Work's own files and Chrome's files.
#[derive(Debug, Clone, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent facts read from different places; the state order combines them"
)]
pub(crate) struct Facts {
    pub package: bool,
    /// The package's own problem text (shown as the label when there is no usable package).
    pub package_text: String,
    pub set_up: bool,
    pub browser_issue: Option<BrowserIssue>,
    pub enabled: bool,
    pub connected: bool,
    pub store_review: bool,
    pub other_owner: bool,
    pub connecting: bool,
    pub waiting: bool,
    pub helper_up: bool,
    /// The helper was refused because another helper holds its connection (`Owner::helper_conflict`).
    pub helper_conflict: bool,
    pub credential_missing: bool,
    /// The device credential cannot be read for lack of a system keyring (`Owner::keyring_unavailable`).
    pub keyring_unavailable: bool,
    pub paired_once: bool,
    /// The person removed every pairing of this browser in Browser settings. The extension may still be on, so this
    /// is not a lost connection: only a new pairing (Connect) brings the browser back.
    pub removed: bool,
    pub chrome_running: bool,
    pub install_state: Option<String>,
    pub extension_folder: bool,
    pub installed_version: Option<String>,
    pub min_version: Option<String>,
}

/// The oldest Chrome/Edge extension this app works with, whatever the signed release record (`min_version`) says or
/// when there is none: from 0.1.7 the extension admits local addresses with the browser's own site permissions, so the
/// app and the helper send it no network rule. An older extension reads "Update the extension".
pub(super) const EXTENSION_FLOOR: &str = "0.1.7";

impl Facts {
    fn outdated(&self) -> bool {
        let Some(installed) = &self.installed_version else {
            return false;
        };
        let below = |min: &str| {
            vw_vibe_install::support::compare_versions(installed, min)
                == Some(std::cmp::Ordering::Less)
        };
        below(EXTENSION_FLOOR) || self.min_version.as_deref().is_some_and(below)
    }
}

/// Live profile IDs and their reported versions, cleared with the helper connection.
#[derive(Default)]
pub(super) struct ConnectedProfiles(std::sync::Mutex<Vec<(String, Option<String>)>>);
impl ConnectedProfiles {
    pub(super) fn remove_browser(&self, browser: super::agent_browser::Browser) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(profile, _)| !browser.owns_profile(profile));
    }
    pub(super) fn observe(&self, data: &serde_json::Value) {
        let profiles = data["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["connected"] == true)
            .filter_map(|p| {
                Some((
                    p["profileId"].as_str()?.to_owned(),
                    p["extensionVersion"].as_str().map(str::to_owned),
                ))
            })
            .collect();
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = profiles;
    }
    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
    pub(super) fn snapshot(&self) -> Vec<(String, Option<String>)> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Choose the version for the card from the live profiles or the on-disk profiles.
pub(super) fn installed_version(
    config: &super::setup::ChromeControlConfig,
    profiles: &[std::path::PathBuf],
    extension_id: &str,
    connected: bool,
    live: &[(String, Option<String>)],
) -> Option<String> {
    let versions: Vec<String> = if connected {
        // No folder can stand in for a connected extension that did not report its version.
        live.iter()
            .filter_map(|(_, version)| version.clone())
            .collect()
    } else {
        profiles
            .iter()
            .filter_map(|p| super::probe::version_for(config, p, extension_id))
            .collect()
    };
    // Every connected profile must meet the minimum. When disconnected, a newer dormant
    // profile must not conceal an older installation in another profile.
    versions
        .into_iter()
        .filter(|v| vw_vibe_install::support::compare_versions(v, v).is_some())
        .min_by(|a, b| {
            vw_vibe_install::support::compare_versions(a, b).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// The state, most blocking first. `probe` reads Chrome's preferences; it runs only when no live state decided.
pub(crate) fn state(facts: &Facts, probe: impl FnOnce() -> Probe) -> AddonState {
    use AddonState as S;
    if !facts.package {
        return S::Unavailable;
    }
    match facts.browser_issue {
        Some(BrowserIssue::Missing) => return S::ChromeMissing,
        Some(BrowserIssue::Sandboxed) => return S::ChromeSandboxed,
        Some(BrowserIssue::DifferentBrowser) => return S::OtherBrowser,
        None => {}
    }
    if !facts.set_up {
        return S::NotSetUp;
    }
    if facts.credential_missing {
        return S::OwnerCredentialMissing;
    }
    if !facts.enabled {
        return S::SwitchedOff;
    }
    if facts.connected {
        return if facts.outdated() {
            S::UpdateNeeded
        } else {
            S::Connected
        };
    }
    if facts.store_review {
        return S::InReview;
    }
    if facts.other_owner {
        return S::OtherOwner;
    }
    // A down helper keeps its own states (Connect cannot help while another helper holds the connection).
    if facts.removed && !facts.waiting && facts.helper_up {
        if facts.extension_folder {
            match probe() {
                Probe::AwaitingEnable => return S::EnableInChrome,
                Probe::Off => return S::TurnedOffInChrome,
                Probe::NotOff | Probe::Unknown => {}
            }
        }
        return S::NotConnected;
    }
    if facts.connecting {
        if facts.paired_once && facts.extension_folder {
            match probe() {
                Probe::AwaitingEnable => return S::EnableInChrome,
                Probe::Off => return S::TurnedOffInChrome,
                Probe::NotOff | Probe::Unknown => {}
            }
        }
        return S::Connecting;
    }
    if facts.waiting {
        return S::NotPaired;
    }
    if !facts.helper_up && facts.keyring_unavailable {
        return S::OwnerKeyringUnavailable;
    }
    if !facts.helper_up && facts.helper_conflict {
        return S::OtherHelperRunning;
    }
    if !facts.helper_up {
        return S::HelperNotRunning;
    }
    let external = facts.install_state.as_deref() == Some("external");
    // Chrome's own reason decides, not how the extension was added: an external install the person enabled and
    // later turned off is "Turn on in Chrome", one still waiting for Chrome's "added" prompt is "Enable in Chrome".
    if facts.extension_folder {
        match probe() {
            Probe::AwaitingEnable => return S::EnableInChrome,
            Probe::Off => return S::TurnedOffInChrome,
            Probe::NotOff | Probe::Unknown => {}
        }
    }
    if facts.install_state.as_deref() == Some("restartChrome") {
        return S::RestartChrome;
    }
    if facts.outdated() {
        return S::UpdateNeeded;
    }
    if external && !facts.paired_once {
        return S::EnableInChrome;
    }
    if !facts.extension_folder {
        return S::ExtensionNotInstalled;
    }
    if facts.paired_once {
        return if facts.chrome_running {
            S::Connecting
        } else {
            S::ChromeClosed
        };
    }
    S::NotConnected
}

#[cfg(test)]
#[path = "addon_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "addon_recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
#[path = "addon_floor_tests.rs"]
mod floor_tests;
