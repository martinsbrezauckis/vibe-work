//! Typed helper connection observations. Owner supervisors project protocol facts here; views alone render labels.
use serde::{Deserialize, Serialize};

/// The helper's pairing observation, independent of display wording and setup availability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "ChromePairingState"))]
pub enum PairingState {
    /// No observation yet.
    #[default]
    Unknown,
    /// No live profile.
    Disconnected,
    /// The extension requests this app's identity.
    Waiting,
    /// Pairing succeeded; adapter registration is pending.
    Connecting,
    /// A live registered profile.
    Paired,
    /// The extension holds a different app identity.
    OtherOwner,
    /// Browser policy forbids this connection.
    Blocked,
    /// The optional owned Firefox window is open.
    WindowOpen,
    /// The optional owned Firefox window is closed.
    WindowClosed,
    /// The optional owned Firefox browser is absent.
    NotInstalled,
}

impl PairingState {
    /// Project trusted helper facts for either browser. A pairing click never proves a live profile.
    pub(super) fn observe(code: &str, connected: bool, waiting: bool) -> Self {
        match code {
            "OWNER_IDENTITY_MISMATCH" | "OWNER_CONFLICT" => Self::OtherOwner,
            "OWNER_CONNECTING" => Self::Connecting,
            _ if connected => Self::Paired,
            _ if waiting => Self::Waiting,
            _ => Self::Disconnected,
        }
    }

    /// Presentation only: consumers use the enum, never these words.
    pub(super) fn label(self, browser: &str) -> String {
        match self {
            Self::Unknown => String::new(),
            Self::Disconnected => format!("Your {browser} is not connected."),
            Self::Waiting => format!("Waiting for you to pair in {browser}"),
            Self::Connecting => "Connecting".into(),
            Self::Paired => "Paired".into(),
            Self::OtherOwner => {
                format!("{browser} is paired with another Vibe Work. Pair again in {browser}.")
            }
            Self::Blocked => "Blocked by your organization".into(),
            Self::WindowOpen => "Window open".into(),
            Self::WindowClosed => "Window closed".into(),
            Self::NotInstalled => "Not installed".into(),
        }
    }

    /// The owned-window helper protocol predates typed app views; parse it only at this boundary.
    pub(super) fn firefox_window(value: &str) -> Option<Self> {
        match value {
            "Window open" => Some(Self::WindowOpen),
            "Window closed" => Some(Self::WindowClosed),
            "Not installed" => Some(Self::NotInstalled),
            _ => None,
        }
    }
}
