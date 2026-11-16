//! Browser identity at the enrollment boundary: helper scopes, pending clicks, state and the selected launcher.
//! Depends on enrollment, owner state and browser launchers; used by Connect and the owner pairing observer.
use std::sync::{Mutex, atomic::AtomicBool};

use crate::core::Shared;
use vw_platform::sync::LockRecover;

/// The browser whose own Connect may consume a waiting report or ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Browser {
    Chrome,
    Edge,
}

impl Browser {
    /// Helpers without browser support use a bare Chrome UUID; scoped tickets must name exactly this browser.
    pub(super) fn raw_profile(self, profile: &str) -> Option<&str> {
        let raw = match self {
            Self::Chrome => profile.strip_prefix("chrome:").unwrap_or(profile),
            Self::Edge => profile.strip_prefix("edge:")?,
        };
        super::enroll::uuid_shape(raw).then_some(raw)
    }

    /// Pairing reports from older Chrome-only helpers may omit a profile label. Foreign scopes never become Chrome.
    pub(super) fn from_report(profile: Option<&str>) -> Option<Self> {
        match profile {
            Some(id) if super::edge_state::chrome_profile(id) => Some(Self::Chrome),
            Some(id) if cfg!(feature = "your-edge") && id.starts_with("edge:") => Some(Self::Edge),
            None if !cfg!(any(feature = "your-edge", feature = "your-firefox")) => {
                Some(Self::Chrome)
            }
            _ => None,
        }
    }

    pub(super) fn enroll(self, shared: &Shared) -> &Mutex<super::enroll::Enroll> {
        match self {
            Self::Chrome => &shared.chrome_control.owner.enroll,
            Self::Edge => &shared.chrome_control.owner.edge_enroll,
        }
    }

    pub(super) fn requested(self, shared: &Shared) -> &AtomicBool {
        match self {
            Self::Chrome => &shared.chrome_control.connect_requested,
            Self::Edge => &shared.chrome_control.edge_connect_requested,
        }
    }

    /// The helper protocol's browser word (`owner.pairingPage.open`).
    pub(super) fn helper_id(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
        }
    }

    /// The helper reported this browser's extension as paired with another Vibe Work.
    pub(super) fn other_owner(self, shared: &Shared) -> bool {
        let pairing = match self {
            Self::Chrome => &shared.chrome_control.owner.pairing,
            Self::Edge => &shared.chrome_control.owner.edge_pairing,
        };
        pairing.lock_or_recover().0 == super::PairingState::OtherOwner
    }

    pub(super) fn other(self) -> Self {
        match self {
            Self::Chrome => Self::Edge,
            Self::Edge => Self::Chrome,
        }
    }

    pub(super) fn open(self, shared: &Shared, url: &str) -> Result<(), String> {
        let config = &shared.chrome_control.config;
        match self {
            Self::Chrome => super::browser::open(config, url),
            Self::Edge => super::browser::open_edge(config, url),
        }
    }
}
