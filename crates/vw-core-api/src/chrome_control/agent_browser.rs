//! Browser identity for agent calls and add-on facts. Uses owned pairing/profile state, never page data.
//! The gateway preserves explicit profile ids and a chat's binding; discovery prefers the only paired browser.
use super::{AddonState, AddonView};
use crate::core::Shared;
use serde_json::Value;
use std::sync::PoisonError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Browser {
    Chrome,
    Edge,
}

impl Browser {
    /// Connection history is a display/reconnect fact, never permission to act.
    pub(crate) fn was_paired(self, shared: &Shared) -> bool {
        let chrome = &shared.chrome_control;
        let paired = match self {
            Self::Chrome => &chrome.paired_once,
            Self::Edge => &chrome.edge_paired_once,
        };
        paired.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Remembered pairing may reconnect silently unless a fresh enrollment requires the person.
    pub(crate) fn reconnectable(self, shared: &Shared) -> bool {
        let owner = &shared.chrome_control.owner;
        let enroll = match self {
            Self::Chrome => &owner.enroll,
            Self::Edge => &owner.edge_enroll,
        };
        self.was_paired(shared)
            && !enroll
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .waiting_reported()
    }
    pub(crate) fn from_profile(profile: &str) -> Self {
        if cfg!(feature = "your-edge") && profile.starts_with("edge:") {
            Self::Edge
        } else {
            Self::Chrome
        }
    }
    /// File/version facts retain browser identity even in builds with another browser's module disabled.
    pub(super) fn owns_profile(self, profile: &str) -> bool {
        match self {
            Self::Chrome => !profile.starts_with("edge:") && !profile.starts_with("firefox:"),
            Self::Edge => profile.starts_with("edge:"),
        }
    }
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Chrome => vw_vocabulary::texts::CHROME_NAME,
            Self::Edge => vw_vocabulary::texts::EDGE_NAME,
        }
    }
    pub(crate) fn text(self, text: &str) -> String {
        match self {
            Self::Chrome => text.into(),
            Self::Edge => text
                .replace("Google Chrome", "Microsoft Edge")
                .replace("Chrome", "Edge"),
        }
    }
    pub(super) fn paired(self, shared: &Shared) -> bool {
        let owner = &shared.chrome_control.owner;
        let pairing = match self {
            Self::Chrome => &owner.pairing,
            Self::Edge => &owner.edge_pairing,
        };
        pairing.lock().unwrap_or_else(PoisonError::into_inner).0 == "Paired"
    }
    pub(crate) fn addon(self, shared: &Shared) -> AddonView {
        if self == Self::Chrome {
            return super::commands::addon(shared);
        }
        let view = super::commands::view(shared);
        view.edge.and_then(|edge| edge.addon).unwrap_or_else(|| {
            let mut view = super::addon::view(AddonState::Unavailable, super::setup::ABSENT);
            view.label = self.text(&view.label);
            view.instruction = self.text(&view.instruction);
            view
        })
    }
}

pub(crate) fn selected(shared: &Shared, session: &str, tool: &str, args: &Value) -> Browser {
    if let Some(profile) = super::receipt::profile(shared, tool, args) {
        return Browser::from_profile(&profile);
    }
    if let Some(profile) = shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(session)
    {
        return Browser::from_profile(profile);
    }
    if cfg!(feature = "your-edge")
        && !Browser::Chrome.paired(shared)
        && (Browser::Edge.paired(shared)
            || (Browser::Edge.was_paired(shared)
                && (!Browser::Chrome.was_paired(shared)
                    || (super::installation::profiles(shared, Browser::Chrome).is_empty()
                        && !super::installation::profiles(shared, Browser::Edge).is_empty()))))
    {
        Browser::Edge
    } else {
        Browser::Chrome
    }
}

/// Edge's own state uses the shared switch/package and only Edge's live pairing/version observations.
/// Chrome's on-disk installation or process probe cannot describe Edge.
pub(super) fn edge_addon(shared: &Shared, setup: &super::setup::ChromeSetupView) -> AddonView {
    use AddonState as S;
    if let Some(reason) = super::edge_policy::refusal(&shared.chrome_control.config) {
        return AddonView {
            state: S::Unavailable, label: reason.into(), action: None, step: None,
            extension_version: super::installation::version(shared, Browser::Edge),
            instruction: "Edge is blocked by your organization's browser policy. Ask the person to contact their administrator to allow this extension and native messaging.".into(),
        };
    }
    let chrome = &shared.chrome_control;
    let live: Vec<_> = chrome
        .owner
        .connected_profiles
        .snapshot()
        .into_iter()
        .filter(|(id, _)| Browser::Edge.owns_profile(id))
        .collect();
    let profiles = super::installation::profiles(shared, Browser::Edge);
    let installed = super::addon::installed_version(
        &profiles,
        &chrome.config.extension_id,
        setup.pairing == "Paired",
        &live,
    );
    let min = shared
        .config
        .vibe_install
        .as_ref()
        .and_then(vw_vibe_install::VibeInstaller::extension_min_version);
    let outdated = installed
        .as_deref()
        .zip(min.as_deref())
        .is_some_and(|(installed, min)| {
            vw_vibe_install::support::compare_versions(installed, min)
                == Some(std::cmp::Ordering::Less)
        });
    let state = if !setup.package {
        S::Unavailable
    } else if setup.state == super::setup::SetupState::RepairRequired {
        S::HelperSetupRequired
    } else if !chrome.enabled() {
        S::SwitchedOff
    } else if setup.pairing == "Paired" {
        if outdated {
            S::UpdateNeeded
        } else {
            S::Connected
        }
    } else if setup.state == super::setup::SetupState::NotSetUp {
        S::NotSetUp
    } else if setup.store_review {
        S::InReview
    } else if setup.pairing.contains("another Vibe Work") {
        S::OtherOwner
    } else if setup.pairing == "Connecting" {
        S::Connecting
    } else if setup.pairing.contains("Waiting for you to pair") {
        S::NotPaired
    } else if !chrome.owner.is_running() {
        S::HelperNotRunning
    } else if !profiles.is_empty()
        && chrome.edge_prober.probe(
            profiles,
            &chrome.config.extension_id,
            std::time::Instant::now(),
        ) == super::probe::Probe::Off
    {
        S::TurnedOffInChrome
    } else if chrome
        .edge_paired_once
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        if chrome.config.launcher.is_none() && super::browser::edge_running() {
            S::Connecting
        } else {
            S::ChromeClosed
        }
    } else {
        S::NotConnected
    };
    let package_text = if chrome.config.available() {
        &setup.pairing
    } else {
        super::setup::absent_for(true)
    };
    let mut view = super::addon::view(state, package_text);
    view.extension_version = super::installation::version(shared, Browser::Edge);
    super::addon::reconnecting(&mut view, Browser::Edge.reconnectable(shared));
    view.label = Browser::Edge.text(&view.label);
    view.instruction = Browser::Edge.text(&view.instruction);
    view.step = view.step.map(|step| Browser::Edge.text(&step));
    view
}
