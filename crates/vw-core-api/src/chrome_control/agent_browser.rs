//! Browser identity for agent calls and add-on facts. Uses owned pairing/profile state, never page data.
//! The gateway preserves explicit profile ids and a chat's binding; discovery prefers the only paired browser.
use super::{AddonState, AddonView, PairingState};
use crate::core::Shared;
use serde_json::Value;
use std::sync::PoisonError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Browser {
    Chrome,
    Edge,
    Safari,
    Firefox,
}

impl Browser {
    /// Connection history is a display/reconnect fact, never permission to act.
    pub(crate) fn was_paired(self, shared: &Shared) -> bool {
        let chrome = &shared.chrome_control;
        let paired = match self {
            Self::Chrome => &chrome.paired_once,
            Self::Edge => &chrome.edge_paired_once,
            Self::Safari | Self::Firefox => return self.paired(shared),
        };
        paired.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Remembered pairing may reconnect silently unless a fresh enrollment requires the person.
    pub(crate) fn reconnectable(self, shared: &Shared) -> bool {
        let owner = &shared.chrome_control.owner;
        let enroll = match self {
            Self::Chrome => &owner.enroll,
            Self::Edge => &owner.edge_enroll,
            Self::Safari | Self::Firefox => return false,
        };
        self.was_paired(shared)
            && !enroll
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .waiting_reported()
    }
    pub(crate) fn from_profile(profile: &str) -> Self {
        if cfg!(feature = "your-safari") && (profile == "safari" || profile.starts_with("safari:"))
        {
            return Self::Safari;
        }
        if cfg!(feature = "your-firefox")
            && (profile.starts_with("firefox:")
                || profile.eq_ignore_ascii_case("firefox")
                || profile.eq_ignore_ascii_case(vw_vocabulary::texts::FIREFOX_NAME))
        {
            return Self::Firefox;
        }
        if cfg!(feature = "your-edge")
            && (profile.starts_with("edge:")
                || profile.eq_ignore_ascii_case("edge")
                || profile.eq_ignore_ascii_case(vw_vocabulary::texts::EDGE_NAME))
        {
            Self::Edge
        } else {
            Self::Chrome
        }
    }
    /// File/version facts retain browser identity even in builds with another browser's module disabled.
    pub(super) fn owns_profile(self, profile: &str) -> bool {
        match self {
            Self::Chrome => {
                !profile.starts_with("edge:")
                    && !profile.starts_with("firefox:")
                    && !profile.starts_with("safari:")
            }
            Self::Edge => profile.starts_with("edge:"),
            Self::Safari => profile.starts_with("safari:"),
            Self::Firefox => profile.starts_with("firefox:"),
        }
    }
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Safari => "safari",
            Self::Firefox => "firefox",
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Chrome => vw_vocabulary::texts::CHROME_NAME,
            Self::Edge => vw_vocabulary::texts::EDGE_NAME,
            Self::Safari => vw_vocabulary::texts::SAFARI_NAME,
            Self::Firefox => vw_vocabulary::texts::FIREFOX_NAME,
        }
    }
    pub(crate) fn short_name(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Safari => "Safari",
            Self::Firefox => "Firefox",
        }
    }
    /// Render fixed templates at the display boundary; browser names in dynamic text are never rewritten.
    pub(super) fn words(self, template: &str) -> String {
        template
            .replace(
                "{download}",
                match self {
                    Self::Chrome => "google.com/chrome",
                    Self::Edge => "microsoft.com/edge",
                    Self::Safari => "apple.com/safari",
                    Self::Firefox => "mozilla.org/firefox",
                },
            )
            .replace(
                "{fullBrowser}",
                match self {
                    Self::Chrome => "Google Chrome",
                    Self::Edge => "Microsoft Edge",
                    Self::Safari => "Safari",
                    Self::Firefox => "Mozilla Firefox",
                },
            )
            .replace("{browser}", self.short_name())
            .replace("{yourBrowser}", self.name())
    }
    pub(crate) fn text(self, text: &str) -> String {
        super::browser_text::render(self, text)
    }
    pub(super) fn paired(self, shared: &Shared) -> bool {
        let owner = &shared.chrome_control.owner;
        let pairing = match self {
            Self::Chrome => &owner.pairing,
            Self::Edge => &owner.edge_pairing,
            Self::Firefox => {
                return owner
                    .firefox_pairing
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
                    == PairingState::WindowOpen;
            }
            Self::Safari => {
                return cfg!(feature = "your-safari")
                    && owner
                        .connected_profiles
                        .snapshot()
                        .iter()
                        .any(|(id, _)| self.owns_profile(id));
            }
        };
        pairing.lock().unwrap_or_else(PoisonError::into_inner).0 == PairingState::Paired
    }
    pub(crate) fn enabled(self, shared: &Shared) -> bool {
        if self == Self::Safari {
            super::safari::enabled(shared)
        } else if self == Self::Edge {
            cfg!(feature = "your-edge") && shared.chrome_control.edge_enabled()
        } else {
            shared.chrome_control.enabled()
        }
    }
    pub(crate) fn addon(self, shared: &Shared) -> AddonView {
        if self == Self::Safari {
            return super::safari::view(shared).map_or_else(
                || {
                    super::addon::view_for(
                        AddonState::Unavailable,
                        &self.words(
                            "{yourBrowser} is unavailable in this build. Use an available browser.",
                        ),
                        self,
                    )
                },
                |v| v.addon,
            );
        }
        if self == Self::Firefox {
            return super::firefox_commands::addon(shared);
        }
        if self == Self::Chrome {
            return super::commands::addon(shared);
        }
        let view = super::commands::view(shared);
        view.edge.and_then(|edge| edge.addon).unwrap_or_else(|| {
            super::addon::view_for(
                AddonState::Unavailable,
                super::setup::absent_for(true),
                self,
            )
        })
    }
}

pub(crate) fn selected(shared: &Shared, session: &str, tool: &str, args: &Value) -> Browser {
    if super::safari::selected(shared, session, tool, args) {
        return Browser::Safari;
    }
    if tool == "chrome_status"
        && let Some(profile) = args["profileId"].as_str()
    {
        return Browser::from_profile(profile);
    }
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
    if super::safari::enabled(shared)
        && (!shared.chrome_control.any_enabled()
            || (!Browser::Chrome.paired(shared)
                && !Browser::Edge.paired(shared)
                && Browser::Safari.paired(shared)))
    {
        return Browser::Safari;
    }
    if cfg!(feature = "your-firefox")
        && Browser::Firefox.paired(shared)
        && !Browser::Chrome.paired(shared)
        && !Browser::Edge.paired(shared)
    {
        return Browser::Firefox;
    }
    if cfg!(feature = "your-edge")
        && Browser::Edge.enabled(shared)
        && (!Browser::Chrome.enabled(shared) || !Browser::Chrome.paired(shared))
        && (!Browser::Chrome.enabled(shared)
            || Browser::Edge.paired(shared)
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

fn edge_version(
    shared: &Shared,
    setup: &super::setup::ChromeSetupView,
    profiles: &[std::path::PathBuf],
) -> Option<String> {
    let chrome = &shared.chrome_control;
    let live: Vec<_> = chrome
        .owner
        .connected_profiles
        .snapshot()
        .into_iter()
        .filter(|(id, _)| Browser::Edge.owns_profile(id))
        .collect();
    super::addon::installed_version(
        &chrome.config,
        profiles,
        &chrome.config.extension_id,
        setup.pairing_state == PairingState::Paired,
        &live,
    )
}

/// Edge's own state uses its switch and the shared package and only Edge's live pairing/version observations.
/// Chrome's on-disk installation or process probe cannot describe Edge.
pub(super) fn edge_addon(shared: &Shared, setup: &super::setup::ChromeSetupView) -> AddonView {
    use AddonState as S;
    if let Some(problem) = setup
        .addon
        .as_ref()
        .filter(|addon| addon.state == super::AddonState::AccessBlocked)
    {
        return problem.clone();
    }
    if let Some(reason) = super::edge_policy::refusal(&shared.chrome_control.config) {
        return AddonView {
            state: S::Unavailable, label: reason.into(), action: None, step: None,
            extension_version: super::installation::version(shared, Browser::Edge),
            instruction: "Edge is blocked by your organization's browser policy. Ask the person to contact their administrator to allow this extension and native messaging.".into(),
        };
    }
    let chrome = &shared.chrome_control;
    let profiles = super::installation::profiles(shared, Browser::Edge);
    // A failed receipt repair is already diagnosed by setup, as on Chrome's connection path.
    if setup.state == super::setup::SetupState::RepairRequired {
        let mut view = super::addon::view_for(S::HelperSetupRequired, "", Browser::Edge);
        view.extension_version = super::installation::version(shared, Browser::Edge);
        return view;
    }
    let (waiting, connecting) = {
        let enroll = chrome
            .owner
            .edge_enroll
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        (
            enroll.waiting_reported(),
            enroll.connecting(tokio::time::Instant::now()),
        )
    };
    let paired_once = Browser::Edge.was_paired(shared);
    let facts = super::addon::Facts {
        package: setup.package,
        package_text: Browser::Edge.text(&setup.pairing),
        set_up: setup.state != super::setup::SetupState::NotSetUp
            && setup.state != super::setup::SetupState::Absent,
        browser_issue: setup.browser_issue,
        enabled: chrome.edge_enabled(),
        connected: setup.pairing_state == PairingState::Paired,
        store_review: setup.store_review,
        other_owner: setup.pairing_state == PairingState::OtherOwner,
        connecting: connecting || setup.pairing_state == PairingState::Connecting,
        waiting: waiting || setup.pairing_state == PairingState::Waiting,
        credential_missing: chrome
            .owner
            .credential_missing
            .load(std::sync::atomic::Ordering::SeqCst),
        helper_up: !chrome
            .owner
            .tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty(),
        paired_once,
        chrome_running: paired_once
            && setup.pairing_state != PairingState::Paired
            && chrome.edge_enabled()
            && chrome.config.edge_launcher.is_none()
            && super::browser::edge_running(&chrome.config),
        install_state: setup.install_state.clone(),
        extension_folder: !profiles.is_empty() || (chrome.config.is_macos() && paired_once),
        installed_version: edge_version(shared, setup, &profiles),
        min_version: shared
            .config
            .vibe_install
            .as_ref()
            .and_then(vw_vibe_install::VibeInstaller::extension_min_version),
    };
    let state = super::addon::state(&facts, || {
        chrome.edge_prober.probe_for(
            &chrome.config,
            profiles,
            &chrome.config.extension_id,
            std::time::Instant::now(),
        )
    });
    let state = super::installation::observed_state(&chrome.config, state);
    let package_text = if chrome.config.available() {
        &setup.pairing
    } else {
        super::setup::absent_for(true)
    };
    let package_text = Browser::Edge.text(package_text);
    let mut view = super::addon::view_for(state, &package_text, Browser::Edge);
    view.extension_version = super::installation::version(shared, Browser::Edge);
    super::addon::reconnecting_for(
        &mut view,
        Browser::Edge.reconnectable(shared),
        Browser::Edge,
    );
    super::installation::present(&chrome.config, &mut view);
    view
}
