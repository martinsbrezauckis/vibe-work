//! Pure browser tiers derived from chat access modes. Used by the core's action map; no stored permissions.
/// Browser authority derived only from the chat's current access mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// View and navigate without entering data.
    Ask,
    /// Interact with pages and files without secrets or scripts.
    AcceptEdits,
    /// All browser operations; vault consent remains separate.
    FullAccess,
}
impl Tier {
    /// Unknown and planning modes offer no browser authority.
    #[must_use]
    pub fn from_mode(mode: &str) -> Option<Self> {
        match mode {
            "ask" => Some(Self::Ask),
            "accept-edits" | "smart-approve" => Some(Self::AcceptEdits),
            "auto-approve" => Some(Self::FullAccess),
            _ => None,
        }
    }
    /// The access mode label used in refusals (the shared vocabulary's mode name).
    #[must_use]
    pub fn label(self) -> &'static str {
        vw_vocabulary::label(
            "accessModes",
            match self {
                Self::Ask => "ask",
                Self::AcceptEdits => "accept-edits",
                Self::FullAccess => "auto-approve",
            },
        )
    }
    /// The modes that include an operation at this tier, e.g. "Accept edits or Full access".
    #[must_use]
    pub fn required_label(self) -> String {
        let (ask, edits, full) = (
            Self::Ask.label(),
            Self::AcceptEdits.label(),
            Self::FullAccess.label(),
        );
        match self {
            Self::Ask => format!("{ask}, {edits} or {full}"),
            Self::AcceptEdits => format!("{edits} or {full}"),
            Self::FullAccess => full.to_owned(),
        }
    }
}

/// Internal action families used by the core's single capability table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Read pages and diagnostics.
    Read,
    /// Navigate and inspect controls.
    Navigate,
    /// Enter data and handle files.
    Forms,
    /// Submit forms and confirm dialogs.
    Send,
    /// Use vault logins.
    SignIn,
    /// Read and change cookies and storage.
    SiteData,
    /// Execute page code and tools.
    Scripts,
    /// Read, create and rename bookmarks.
    Bookmarks,
    /// Read browsing history.
    History,
    /// Remove browser data.
    RemoveData,
}
/// The internal family needed by one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Session and connection inspection.
    Nothing,
    /// A mapped action family.
    Group(Group),
    /// An unrecognised action; it has no authority.
    Unknown,
}
#[cfg(test)]
#[path = "chrome_tests.rs"]
mod tests;
