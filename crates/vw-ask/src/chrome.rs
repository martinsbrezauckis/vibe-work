//! Chrome Control permissions: the three trust levels the user chooses in Settings ›
//! Browser › Your Chrome, which group of operations each Chrome Control tool belongs to, and what one call needs.
//! Pure: the core stores the level and opens the asks; the window draws them. Depends on serde.
//!
//! Three cumulative levels, raised only in Vibe Work, with one approval per call:
//! - the user picks one level: Basic browser control (default), Advanced, Full: works on your behalf. Each includes
//!   the ones below it. There are no per-group rows and no Never;
//! - the groups are internal: each level includes some; a call of a group above the level asks with the one card;
//! - Bookmarks and Browsing history belong to no level: they ask at every level;
//! - closing the user's own tabs is the separate Settings control `closeUserTabs` (the only "Always" target).
//!
//! The core maps each helper action to these groups: status and task tools need nothing; other actions need their
//! group; a Chrome Control tool this table does not know is asked about (never a silent No) with Allow once and No
//! only, until the table gets a line for it.
/// The trust levels, from least to most. Each includes everything below it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "ChromeLevel"))]
pub enum Level {
    /// Look around and move through sites (the default).
    Browse,
    /// Prepare things: forms, downloads, uploads; the user still sends.
    Fill,
    /// Work on the user's behalf, end to end.
    Act,
}

impl Level {
    /// Every level, in order.
    pub const ALL: [Self; 3] = [Self::Browse, Self::Fill, Self::Act];

    /// The level's id in `vocabulary.json` (group `chromeControl`).
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Browse => "browse",
            Self::Fill => "fill",
            Self::Act => "act",
        }
    }
}

/// One group of Chrome Control's operations (internal: the levels include groups; the cards name them).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "ChromeGroup"))]
pub enum Group {
    /// Read pages, take screenshots, see the open tabs.
    Read,
    /// Go to sites, open and close the tabs Vibe opened, click, scroll, press keys, type into text fields.
    Navigate,
    /// Fill in forms without sending them, download files, upload a file the user picked.
    Forms,
    /// Send forms and confirm: buy, book, delete.
    Send,
    /// Sign in with a vault login the user allowed.
    SignIn,
    /// Read and change cookies and site data.
    SiteData,
    /// Run page scripts.
    Scripts,
    /// Read and change bookmarks (no level includes it).
    Bookmarks,
    /// Read and delete browsing history (no level includes it).
    History,
}

impl Group {
    /// Every row, in the order Settings shows them.
    pub const ALL: [Self; 9] = [
        Self::Read,
        Self::Navigate,
        Self::Forms,
        Self::Send,
        Self::SignIn,
        Self::SiteData,
        Self::Scripts,
        Self::Bookmarks,
        Self::History,
    ];

    /// The lowest level that includes this group; `None` for the "Also asks first" rows.
    #[must_use]
    pub const fn level(self) -> Option<Level> {
        match self {
            Self::Read | Self::Navigate => Some(Level::Browse),
            Self::Forms => Some(Level::Fill),
            Self::Send | Self::SignIn | Self::SiteData | Self::Scripts => Some(Level::Act),
            Self::Bookmarks | Self::History => None,
        }
    }

    /// The group's id in `vocabulary.json` (group `chromeControl`) and on the wire.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Navigate => "navigate",
            Self::Forms => "forms",
            Self::Send => "send",
            Self::SignIn => "signIn",
            Self::SiteData => "siteData",
            Self::Scripts => "scripts",
            Self::Bookmarks => "bookmarks",
            Self::History => "history",
        }
    }

    /// The ask policy kind of a card for this group (`policy.rs`).
    #[must_use]
    pub const fn ask_kind(self) -> &'static str {
        match self {
            Self::Read => "chrome/ask/read",
            Self::Navigate => "chrome/ask/navigate",
            Self::Forms => "chrome/ask/forms",
            Self::Send => "chrome/ask/send",
            Self::SignIn => "chrome/ask/signIn",
            Self::SiteData => "chrome/ask/siteData",
            Self::Scripts => "chrome/ask/scripts",
            Self::Bookmarks => "chrome/ask/bookmarks",
            Self::History => "chrome/ask/history",
        }
    }

    /// The group for a wire id.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id() == id)
    }
}

/// The ask kind of a card for closing one of the user's own tabs (its "Always" sets `closeUserTabs`).
pub const CLOSE_TABS_KIND: &str = "chrome/ask/closeTabs";
/// The ask kind of a card for a Chrome Control tool the table does not know (no session answer, no "Always").
pub const OTHER_KIND: &str = "chrome/ask/other";
/// The first use of the user's Chrome in a chat (session Allow / No only).
pub const SESSION_KIND: &str = "chrome/ask/session";
/// The native confirmation that loosens a row or the level (person only, `policy.rs`).
pub const LOOSEN_KIND: &str = "chrome/control/loosen";
/// Re-confirming a stored raise at app start (the same person-only native authority boundary).
pub const RESTORE_KIND: &str = "chrome/control/restore";

/// What the user chose in Settings › Browser › Your Chrome: one level. The groups are only the mapping of Chrome
/// Control's operations to the levels; the user never sets them one by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChromeSettings {
    /// The trust level.
    pub level: Level,
}

impl Default for Level {
    /// Basic browser control.
    fn default() -> Self {
        Self::Browse
    }
}

impl ChromeSettings {
    /// Whether `group` runs without asking at this level. Bookmarks and History ask at every level.
    #[must_use]
    pub fn allows(self, group: Group) -> bool {
        group.level().is_some_and(|needs| needs <= self.level)
    }

    /// Whether moving to `next` lets Vibe do more (a raise needs the person's native confirmation).
    #[must_use]
    pub fn raises(self, next: Self) -> bool {
        next.level > self.level
    }

    /// The groups `next` adds over these settings (what the native confirmation lists).
    #[must_use]
    pub fn added(self, next: Self) -> Vec<Group> {
        Group::ALL
            .into_iter()
            .filter(|g| next.allows(*g) && !self.allows(*g))
            .collect()
    }
}

/// What one Chrome Control call needs before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Status, task and tab-handover tools: not permissions.
    Nothing,
    /// That group's row.
    Group(Group),
    /// Closing one of the user's own tabs: the `closeUserTabs` control.
    CloseUserTabs,
    /// A tool this table does not know: asked about with Allow once and No.
    Unknown,
}

#[cfg(test)]
#[path = "chrome_tests.rs"]
mod tests;
