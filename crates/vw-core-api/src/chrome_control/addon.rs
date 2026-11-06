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
    /// Vibe Work's Chrome helper is not running.
    HelperNotRunning,
    /// The device credential was deleted outside the app; remove the old registration before connecting.
    OwnerCredentialMissing,
    /// The recorded native registration could not be repaired.
    HelperSetupRequired,
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
    pub credential_missing: bool,
    pub paired_once: bool,
    pub chrome_running: bool,
    pub install_state: Option<String>,
    pub extension_folder: bool,
    pub installed_version: Option<String>,
    pub min_version: Option<String>,
}

impl Facts {
    fn outdated(&self) -> bool {
        match (&self.installed_version, &self.min_version) {
            (Some(installed), Some(min)) => {
                vw_vibe_install::support::compare_versions(installed, min)
                    == Some(std::cmp::Ordering::Less)
            }
            _ => false,
        }
    }
}

/// Live profile IDs and their reported versions, cleared with the helper connection.
#[derive(Default)]
pub(super) struct ConnectedProfiles(std::sync::Mutex<Vec<(String, Option<String>)>>);
impl ConnectedProfiles {
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
            .filter_map(|p| super::probe::installed_version(p, extension_id))
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

/// Chrome's presentation of the shared state table.
pub(crate) fn view(state: AddonState, package_text: &str) -> AddonView {
    view_for(state, package_text, super::agent_browser::Browser::Chrome)
}

/// Chrome's reconnect presentation.
pub(super) fn reconnecting(view: &mut AddonView, paired_once: bool) {
    reconnecting_for(view, paired_once, super::agent_browser::Browser::Chrome);
}

/// The card's label, action and the person's instruction for `state`.
#[allow(
    clippy::too_many_lines,
    reason = "one row per state: the table is the point"
)]
pub(crate) fn view_for(
    state: AddonState,
    package_text: &str,
    browser: super::agent_browser::Browser,
) -> AddonView {
    use AddonAction as A;
    use AddonState as S;
    let (label, action, instruction): (&str, Option<A>, &str) = match state {
        S::Unavailable => (
            package_text,
            None,
            "This Vibe Work has no {browser} connection. Tell the person {yourBrowser} needs a Vibe Work build that includes it.",
        ),
        S::NotSetUp if browser == super::agent_browser::Browser::Edge => (
            "Not connected",
            Some(A::Connect),
            "{yourBrowser} is not set up. Ask the person to click Set up… in Settings, Browser, {yourBrowser}.",
        ),
        S::NotSetUp => (
            "Not connected",
            Some(A::Connect),
            "{yourBrowser} is not set up. Ask the person to click Connect {browser}.",
        ),
        S::ChromeMissing => (
            "{fullBrowser} is not installed",
            None,
            "{fullBrowser} is not installed. Ask the person to install {fullBrowser} from {download}.",
        ),
        S::ChromeSandboxed => (
            "Snap or Flatpak {browser} is unsupported",
            None,
            "{browser} here is a Snap or Flatpak package, which cannot reach Vibe Work. Ask the person to install {fullBrowser} from {download}.",
        ),
        S::OtherBrowser => (
            "{fullBrowser} required",
            None,
            "Only another Chromium browser was found; {yourBrowser} needs {fullBrowser}. Ask the person to install {fullBrowser} from {download}.",
        ),
        S::SwitchedOff => (
            "Off",
            Some(A::TurnOn),
            "{yourBrowser} is switched off in Vibe Work. Ask the person to turn it on in Settings, Browser, {yourBrowser}.",
        ),
        S::InReview => (
            "Extension in review",
            None,
            "The Vibe Work extension is in Google's review and cannot be added yet. Tell the person {yourBrowser} works once it is listed.",
        ),
        S::OtherOwner => (
            "Paired with another Vibe Work",
            Some(A::OpenPairingPage),
            "{browser} is paired with another Vibe Work. Ask the person to click Open pairing page and choose Replace paired Vibe Work in {browser}.",
        ),
        S::Connecting => (
            "Connecting",
            None,
            "{browser} is connecting to Vibe Work. Ask the person to wait a few seconds, then try again.",
        ),
        S::NotPaired => (
            "Pair in {browser}",
            Some(A::OpenPairingPage),
            "The Vibe Work extension waits to be paired. Ask the person to click Pair Vibe Work on its page in {browser} after checking that the code matches the one in Vibe Work.",
        ),
        S::HelperSetupRequired => (
            "Helper setup required",
            Some(A::SetupAgain),
            "{browser}'s helper registration is missing or changed and could not be repaired. Ask the person to click Set up again in Settings, Browser, {yourBrowser}.",
        ),
        S::OwnerCredentialMissing if browser == super::agent_browser::Browser::Edge => (
            "Browser credential missing",
            Some(A::Remove),
            "Vibe Work's browser credential is missing. Ask the person to click Remove… in Settings, Browser, {yourBrowser}, then click Set up….",
        ),
        S::OwnerCredentialMissing => (
            "Browser credential missing",
            Some(A::Remove),
            "Vibe Work's browser credential is missing. Ask the person to click Remove… in Settings, Browser, then Connect {browser}.",
        ),
        S::HelperNotRunning if browser == super::agent_browser::Browser::Edge => (
            "Not running",
            Some(A::Connect),
            "Vibe Work's {browser} connection is not running. Ask the person to click Finish setting up… in Settings, Browser, {yourBrowser}.",
        ),
        S::HelperNotRunning => (
            "Not running",
            Some(A::Connect),
            "Vibe Work's {browser} helper is not running. Ask the person to click Connect {browser}.",
        ),
        S::TurnedOffInChrome => (
            "Turn on in {browser}",
            None,
            "The Vibe Work extension is turned off in {browser}, for example after an update that asked for new permissions. Ask the person to turn it on in {browser}'s Extensions page.",
        ),
        S::RestartChrome => (
            "Restart {browser}",
            None,
            "{browser} must restart to find the extension. Ask the person to quit and reopen {browser}.",
        ),
        S::UpdateNeeded => (
            "Update the extension",
            None,
            "The Vibe Work extension in {browser} is older than this Vibe Work needs. Ask the person to update it: {browser} updates extensions by itself within a few hours, or at once with Update on {browser}'s Extensions page in Developer mode.",
        ),
        S::ChromeClosed => (
            "Connected when {browser} is open",
            None,
            "{yourBrowser} is paired but {browser} is not open. Ask the person to open {fullBrowser}; it connects by itself.",
        ),
        S::EnableInChrome => (
            "Enable in {browser}",
            None,
            "{browser} asks to enable the newly added Vibe Work extension. Ask the person to click Enable extension in {browser}.",
        ),
        S::ExtensionNotInstalled => (
            "Add to {browser}",
            Some(A::Connect),
            "The Vibe Work extension is not in {browser} yet. Ask the person to click Connect {browser} and add the extension in {browser}.",
        ),
        S::NotConnected => (
            "Not connected",
            Some(A::Connect),
            "{browser} is not connected to Vibe Work, perhaps because {browser} is closed. Ask the person to open {fullBrowser} or click Connect {browser}.",
        ),
        S::Connected => ("Connected", None, "{yourBrowser} is connected."),
    };
    let step: Option<String> = match state {
        S::OwnerCredentialMissing if browser == super::agent_browser::Browser::Edge => {
            Some("Remove, then Set up…".into())
        }
        S::OwnerCredentialMissing => Some(browser.words("Remove, then Connect {browser}")),
        S::ChromeMissing | S::ChromeSandboxed | S::OtherBrowser => {
            Some(browser.words("Install {fullBrowser}"))
        }
        S::EnableInChrome if cfg!(target_os = "macos") => {
            Some(browser.words("Click Action required → New extension added (Vibe Work)."))
        }
        S::EnableInChrome => Some(browser.words("Click Enable extension in {browser}.")),
        S::RestartChrome if cfg!(target_os = "macos") => {
            Some(browser.words("Quit {browser} with Cmd+Q and open it again."))
        }
        S::RestartChrome => Some(browser.words("Close every {browser} window and open it again.")),
        _ => None,
    };
    let instruction = match state {
        S::EnableInChrome => format!(
            "{{browser}} asks to enable the newly added Vibe Work extension. Ask the person to follow this step: {}",
            step.as_deref().unwrap_or_default()
        ),
        S::RestartChrome => format!(
            "{{browser}} must restart to find the extension. Ask the person to follow this step: {}",
            step.as_deref().unwrap_or_default()
        ),
        _ => instruction.to_owned(),
    };
    AddonView {
        state,
        extension_version: None,
        step,
        label: browser.words(label),
        action,
        // Names come from the vocabulary (one source for the card, the agent and the docs).
        instruction: browser.words(&instruction),
    }
}

/// A paired browser reconnects automatically. Sleep and blocked extension cleanup look identical from this
/// side of the port; leave time for reconnection before asking the person to recover a persistent failure.
pub(super) fn reconnecting_for(
    view: &mut AddonView,
    paired_once: bool,
    browser: super::agent_browser::Browser,
) {
    if paired_once && view.state == AddonState::Connecting {
        view.label = browser.words("Turn on in {browser}");
        view.action = None;
        view.instruction = browser.words("{browser}'s Vibe Work connection was lost and is reconnecting automatically. Retry this step in a few seconds. Only if the connection stays down, ask the person to open {browser}'s Extensions page and turn on Vibe Work if it is off, or restart {browser} if it is already on.");
        view.step = Some(browser.words("Open {browser} extensions → Vibe Work"));
    }
}

#[cfg(test)]
#[path = "addon_tests.rs"]
mod tests;
