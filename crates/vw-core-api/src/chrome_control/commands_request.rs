//! The Settings and setup requests the UI sends for browser control (`chromeControl`).

/// What the window asks of Chrome Control.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChromeControlRequest {
    /// Availability, connection and chat work states.
    Read {},
    /// Select this chat's exact profile after the old task acknowledges release.
    SelectProfile {
        /// Owning chat or New chat draft.
        session_id: String,
        /// Exact paired profile identity.
        profile_id: String,
    },
    /// Choose this browser's default for new chat bindings.
    SetDefaultProfile {
        /// Exact paired identity.
        profile_id: String,
    },
    /// Ask in each chat again: clear this browser's default when it is still this identity.
    ClearDefaultProfile {
        /// The current default's exact paired identity.
        profile_id: String,
    },
    /// Enable or fence only this profile.
    SetProfileEnabled {
        /// Exact paired identity.
        profile_id: String,
        /// Whether the profile is available.
        enabled: bool,
    },
    /// Reconnect a previously paired identity without choosing another account.
    ConnectProfile {
        /// Exact paired identity.
        profile_id: String,
    },
    /// Remove this app pairing, leaving browser data and sign-ins intact.
    RemoveProfile {
        /// Exact paired identity.
        profile_id: String,
    },
    /// Claim one person-shared browser context item for the composer.
    TakeSelection {
        /// The pending selection identity.
        selection_id: String,
    },
    /// The per-chat agent control frame colour (kept across restarts, the same in every browser that frames the
    /// chat's work). Every chat starts at the one default colour, index 0.
    SetFrameColour {
        /// The chat.
        session_id: String,
        /// Palette index 0-7.
        colour: u8,
    },
    /// List connected tabs without opening a chat window.
    Tabs {},
    /// Read the exact tab selected by the person.
    TabContext {
        /// Connected profile identity.
        profile_id: String,
        /// Exact tab identity.
        tab_id: u32,
        /// Receiving chat or New chat's draft owner; omitted for a read-only context lookup.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
    },
    /// Turn Chrome tools on or off.
    SetEnabled {
        /// Whether Chrome tools are available.
        enabled: bool,
    },
    /// Turn Edge tools on or off without changing Chrome.
    SetEdgeEnabled {
        /// Whether Edge tools are available.
        enabled: bool,
    },
    /// Turn Firefox tools on or off without changing the other browsers.
    SetFirefoxEnabled {
        /// Whether Firefox tools are available.
        enabled: bool,
    },
    /// Close Firefox and remove its connection.
    RemoveFirefox {},
    /// Legacy command, accepted for older callers: browser address access is always enabled, so it changes nothing.
    SetLocalNetwork {
        /// Ignored legacy value; browser address access is always enabled.
        enabled: bool,
    },
    /// What Set up installs, as an offer the click confirms.
    OfferSetup {},
    /// Set up, confirming the offer the window showed.
    Setup {
        /// The offer's id.
        offer_id: String,
    },
    /// Open the extension's store page in Chrome.
    OpenStore {},
    /// Continue installation after setup, using Chrome's native prompt where supported.
    Connect {},
    /// Offer the shared browser setup with the store opened in Edge.
    OfferEdgeSetup {},
    /// Confirm the one-time Edge setup offer.
    SetupEdge {
        /// The offer shown to the person.
        offer_id: String,
    },
    /// Open the unchanged extension listing in Edge.
    OpenEdgeStore {},
    /// Offer the optional Firefox connection.
    OfferFirefoxSetup {},
    /// Confirm the Firefox setup offer.
    SetupFirefox {
        /// The offer shown to the person.
        offer_id: String,
    },
    /// Open the approved Firefox add-on listing.
    OpenFirefoxStore {},
    /// Open or reuse the app-owned Firefox window.
    OpenFirefox {},
    /// The person closes the app-owned Firefox window.
    CloseFirefox {},
    /// Open Safari setup through the utility embedded in Vibe Work.
    OpenSafariSetup {},
    /// The person's independent Safari availability switch.
    SetSafariEnabled {
        /// Whether the optional Safari connection is enabled.
        enabled: bool,
    },
    /// Unregister only the embedded Safari connection and leave it off.
    RemoveSafari {},
    /// Remove only the Chrome connection.
    Remove {},
    /// Remove only the Edge connection.
    RemoveEdge {},
    /// macOS: read browser folders the system refused again, after the person may have allowed them.
    RecheckProfileNames {},
    /// macOS: open System Settings where the person allows Vibe Work to read Chrome's or Edge's profile names.
    OpenProfileNameSettings {},
}
