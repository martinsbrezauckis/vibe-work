//! The add-on card's words and actions for each state, and the reconnecting and window-closed variants.
use super::{AddonAction, AddonState, AddonView, OTHER_HELPER_RUNNING};

/// The person's step when the extension is off. Chrome also turns it off by itself when an update asks for more
/// permissions; it then stays off until the person confirms Re-enable in Chrome's own dialog.
pub(super) const TURN_ON_STEP: &str = "Open {browser} extensions → Vibe Work and turn it on. If {browser} asks for new permissions, click Re-enable.";

/// Chrome's presentation of the shared state table.
pub(crate) fn view(state: AddonState, package_text: &str) -> AddonView {
    view_for(
        state,
        package_text,
        super::super::agent_browser::Browser::Chrome,
    )
}

/// Chrome's reconnect presentation.
pub(in crate::chrome_control) fn reconnecting(view: &mut AddonView, paired_once: bool) {
    reconnecting_for(
        view,
        paired_once,
        super::super::agent_browser::Browser::Chrome,
    );
}

/// The card's label, action and the person's instruction for `state`.
#[allow(
    clippy::too_many_lines,
    reason = "one row per state: the table is the point"
)]
pub(crate) fn view_for(
    state: AddonState,
    package_text: &str,
    browser: super::super::agent_browser::Browser,
) -> AddonView {
    use AddonAction as A;
    use AddonState as S;
    let (label, action, instruction): (&str, Option<A>, &str) = match state {
        S::Unavailable => (package_text, None, super::super::browser_text::UNAVAILABLE),
        S::NotSetUp => (
            "Not connected",
            Some(A::Connect),
            "{yourBrowser} is not set up. Ask the user to click Connect {browser}.",
        ),
        S::ChromeMissing => (
            "{fullBrowser} is not installed",
            None,
            "{fullBrowser} is not installed. Ask the user to install {fullBrowser} from {download}.",
        ),
        S::ChromeSandboxed => (
            "Snap or Flatpak {browser} is unsupported",
            None,
            "{browser} here is a Snap or Flatpak package, which cannot reach Vibe Work. Ask the user to install {fullBrowser} from {download}.",
        ),
        S::OtherBrowser => (
            "{fullBrowser} required",
            None,
            "Only another Chromium browser was found; {yourBrowser} needs {fullBrowser}. Ask the user to install {fullBrowser} from {download}.",
        ),
        S::SwitchedOff => (
            "Off",
            Some(A::TurnOn),
            super::super::browser_text::SWITCHED_OFF,
        ),
        S::InReview => (
            "Extension in review",
            None,
            "The Vibe Work extension is in Google's review and cannot be added yet. Tell the user {yourBrowser} works once it is listed.",
        ),
        S::OtherOwner => (
            "Paired with another Vibe Work",
            Some(A::OpenPairingPage),
            "{browser} is paired with another Vibe Work. Ask the user to click Open pairing page and choose Replace paired Vibe Work in {browser}.",
        ),
        S::Connecting => (
            "Connecting",
            None,
            "{browser} is connecting to Vibe Work. Ask the user to wait a few seconds, then try again.",
        ),
        S::NotPaired => (
            "Pair in {browser}",
            Some(A::OpenPairingPage),
            "The Vibe Work extension waits to be paired. Ask the user to click Pair Vibe Work on its page in {browser} after checking that the code matches the one in Vibe Work.",
        ),
        S::AccessBlocked => (
            "Blocked by macOS",
            Some(A::Connect),
            "macOS blocked access. Ask the user to open System Settings > Privacy & Security > Files & Folders > Vibe Work, allow {fullBrowser} if listed, then retry Connect {browser}.",
        ),
        S::HelperSetupRequired => (
            "Helper setup required",
            Some(A::SetupAgain),
            "{browser}'s connection setup is missing or changed and could not be repaired. Ask the user to click Set up again in Settings > Browser > {yourBrowser}.",
        ),
        S::OwnerCredentialMissing => (
            "Browser credential missing",
            Some(A::Remove),
            "Vibe Work's browser credential is missing. Ask the user to click Remove… in Settings > Browser, then Connect {browser}.",
        ),
        S::OwnerKeyringUnavailable => (
            "Not running",
            Some(A::Connect),
            vw_vocabulary::texts::CHROME_OWNER_STORE_UNAVAILABLE,
        ),
        S::OtherHelperRunning => (
            "Another Vibe Work helper is running",
            None,
            OTHER_HELPER_RUNNING,
        ),
        S::HelperNotRunning => (
            "Not running",
            Some(A::Connect),
            "Vibe Work's connection to {browser} is not running. Ask the user to click Connect {browser}.",
        ),
        S::TurnedOffInChrome => (
            "Turn on in {browser}",
            None,
            "The Vibe Work extension is turned off in {browser}, for example after an update that asked for new permissions. Ask the user to turn it on in {browser}'s Extensions page and click Re-enable if {browser} asks.",
        ),
        S::RestartChrome => (
            "Restart {browser}",
            None,
            "{browser} must restart to find the extension. Ask the user to quit and reopen {browser}.",
        ),
        S::UpdateNeeded => (
            "Update the extension",
            None,
            "The Vibe Work extension in {browser} is older than this Vibe Work needs. Ask the user to update it: {browser} updates extensions by itself within a few hours, or at once with Update on {browser}'s Extensions page in Developer mode.",
        ),
        S::ChromeClosed => (
            "Connected when {browser} is open",
            None,
            "{yourBrowser} is paired but {browser} is not open. Call chrome_session with action open and profileId {browser}; Vibe Work opens it (card first in Ask). Do not start the browser with process or shell tools: the app's launcher leaves the browser open after Vibe Work quits.",
        ),
        S::EnableInChrome => (
            "Enable in {browser}",
            None,
            "{browser} asks to enable the newly added Vibe Work extension. Ask the user to click Enable extension in {browser}.",
        ),
        S::ExtensionNotInstalled => (
            "Add to {browser}",
            Some(A::Connect),
            "The Vibe Work extension is not in {browser} yet. Ask the user to click Connect {browser} and add the extension in {browser}.",
        ),
        S::NotConnected => (
            "Not connected",
            Some(A::Connect),
            "{browser} is not connected to Vibe Work, perhaps because {browser} is closed. Ask the user to open {fullBrowser} or click Connect {browser}.",
        ),
        S::Connected => ("Connected", None, "{yourBrowser} is connected."),
    };
    let step: Option<String> = match state {
        S::AccessBlocked => Some("Check Files & Folders".into()),
        S::OwnerCredentialMissing => Some(browser.words("Remove, then Connect {browser}")),
        S::OwnerKeyringUnavailable => Some(browser.words(vw_vocabulary::texts::CHROME_OWNER_STORE_UNAVAILABLE)),
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
        S::TurnedOffInChrome => Some(browser.words(TURN_ON_STEP)),
        S::OtherHelperRunning if cfg!(target_os = "macos") => Some(
            "Quit the other Vibe Work copy with Cmd+Q. If none is open, restart the Mac. Vibe Work then connects by itself."
                .into(),
        ),
        S::OtherHelperRunning if cfg!(windows) => Some(
            "Close the other Vibe Work copy. If none is open, restart the computer. Vibe Work then connects by itself."
                .into(),
        ),
        S::OtherHelperRunning => Some(
            "Quit the other Vibe Work copy. If none is open, restart the computer. Vibe Work then connects by itself."
                .into(),
        ),
        _ => None,
    };
    let instruction = match state {
        S::Unavailable => super::super::browser_text::unavailable(browser, package_text),
        S::EnableInChrome => format!(
            "{{browser}} asks to enable the newly added Vibe Work extension. Ask the user to follow this step: {}",
            step.as_deref().unwrap_or_default()
        ),
        S::RestartChrome => format!(
            "{{browser}} must restart to find the extension. Ask the user to follow this step: {}",
            step.as_deref().unwrap_or_default()
        ),
        _ => instruction.to_owned(),
    };
    AddonView {
        state,
        extension_version: None,
        step,
        label: super::super::browser_text::label(browser, &browser.words(label)),
        action,
        // Names come from the vocabulary (one source for the card, the agent and the docs).
        instruction: browser.words(&instruction),
    }
}

/// A paired browser reconnects automatically. Sleep and blocked extension cleanup look identical from this
/// side of the port; leave time for reconnection before asking the person to recover a persistent failure.
pub(in crate::chrome_control) fn reconnecting_for(
    view: &mut AddonView,
    paired_once: bool,
    browser: super::super::agent_browser::Browser,
) {
    if paired_once && view.state == AddonState::Connecting {
        view.label = browser.words("Turn on in {browser}");
        view.action = None;
        view.instruction = browser.words("{browser}'s Vibe Work connection was lost and is reconnecting automatically. Retry this step in a few seconds. Only if the connection stays down, ask the user to open {browser}'s Extensions page and turn on Vibe Work if it is off (clicking Re-enable if {browser} asks), or restart {browser} if it is already on.");
        view.step = Some(browser.words(TURN_ON_STEP));
    }
}

/// A paired browser runs but no window of a paired profile is open (its profile picker, for example), and the
/// extension is on in a paired profile: the person opens a window; nothing needs turning on.
pub(in crate::chrome_control) fn window_closed_for(
    view: &mut AddonView,
    browser: super::super::agent_browser::Browser,
) {
    if view.state == AddonState::Connecting {
        view.label = browser.words("Open a {browser} window");
        view.action = None;
        view.instruction = browser.words("No window of a paired {browser} profile is open, so its Vibe Work extension is not running. Ask the user to open a window of the profile they want in {browser}; if {browser} shows its profile picker, they choose that profile there.");
        view.step = Some(browser.words(WINDOW_STEP));
    }
}

/// A paired browser runs, no paired profile is connected, and its files cannot be read (macOS): the extension is not
/// known to be off, so the card names the likely steps, a closed window or a waiting approval, never Re-enable.
pub(in crate::chrome_control) fn window_or_approval_for(
    view: &mut AddonView,
    browser: super::super::agent_browser::Browser,
) {
    if view.state == AddonState::Connecting {
        view.label = browser.words("Open a {browser} window");
        view.action = None;
        view.instruction = browser.words("No paired {browser} profile is connected. Its window may be closed, or it may wait for the user's approval in Vibe Work. Ask the user to open a window of a paired {browser} profile, or to approve it in Vibe Work; only if the connection stays down, to check in {browser}'s Extensions page that Vibe Work is on.");
        view.step = Some(
            browser
                .words("Open a window of a paired {browser} profile, or approve it in Vibe Work."),
        );
    }
}

/// The person's step when no window of a paired profile is open.
pub(super) const WINDOW_STEP: &str = "Open a window of a paired {browser} profile. If {browser} shows its profile picker, choose that profile.";
