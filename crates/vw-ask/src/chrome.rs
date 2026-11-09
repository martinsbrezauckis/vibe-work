//! Pure browser tiers derived from chat access modes. Used by the core's action map; no stored permissions.
/// Browser authority derived only from the chat's current access mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// View, navigate and choose existing options.
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
    /// Reading secret values requires the chat itself to have Full access, never a tool grant.
    SecretRead(SecretRead),
    /// An unrecognised action; it has no authority.
    Unknown,
}

/// Values that can expose credentials or other private information through a browser read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretRead {
    /// Site cookie values, including session credentials.
    Cookies,
    /// Local or session storage values.
    Storage,
    /// Plain text from the system clipboard.
    Clipboard,
    /// Request or response headers with redaction disabled.
    Headers,
    /// Captured network response bytes that may never have appeared on the page.
    ResponseBody,
}
impl SecretRead {
    /// A refusal explains the risk and tells the agent how the person can authorize the read.
    #[must_use]
    pub fn refusal(self) -> String {
        let (operation, reason) = match self {
            Self::Cookies => ("Reading this site's cookies", "they can sign in as you"),
            Self::Storage => (
                "Reading this site's stored values",
                "they can contain sign-in tokens and private data",
            ),
            Self::Clipboard => (
                "Reading clipboard text",
                "it can contain passwords and other private data",
            ),
            Self::Headers => (
                "Reading unredacted request or response headers",
                "they can contain cookies and sign-in credentials",
            ),
            Self::ResponseBody => (
                "Reading a captured network response body",
                "it can contain sign-in tokens and private data that were never displayed on the page",
            ),
        };
        let full = Tier::FullAccess.label();
        format!(
            "Not run: {operation} needs {full}, because {reason}; ask the person to switch this chat to {full} if the task needs it."
        )
    }
}

/// Whether a known browser operation enters data or sends information. Both gateways use this
/// rule for tool approval; the shared action table separately identifies reads requiring Full access.
#[must_use]
pub fn asks(action: &str, key: Option<&str>) -> bool {
    match action {
        "page.key" => !navigation_key(key.unwrap_or_default()),
        "clipboard.write" | "network.intercept" | "auth.basic" | "auth.fill" | "files.drop"
        | "webmcp.call" | "page.type" | "files.upload" | "page.commit" | "cookies.change"
        | "storage.change" | "raw.execute" | "bookmarks.edit" | "bookmarks.remove"
        | "history.delete" | "page.drag" | "page.dialog.accept" | "page.dialog.prompt" => true,
        _ => false,
    }
}

/// Keys that move, inspect or dismiss without typing or confirming a submission.
#[must_use]
pub fn navigation_key(key: &str) -> bool {
    matches!(
        key,
        "Tab"
            | "Shift+Tab"
            | "Escape"
            | "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
    )
}

/// Existing card verbs, also used to describe exactly what a browser session package covers.
pub const ACTION_TEXT: &str = include_str!("browser_actions.json");

/// Write operations in the package at `tier`, including the lower tier. Vault use still asks separately.
pub fn package_actions(tier: Tier) -> impl Iterator<Item = &'static str> {
    const WRITES: &[(&str, Tier)] = &[
        ("tabs.emulate", Tier::AcceptEdits),
        ("page.type", Tier::AcceptEdits),
        ("page.key", Tier::AcceptEdits),
        ("page.drag", Tier::AcceptEdits),
        ("files.upload", Tier::AcceptEdits),
        ("files.drop", Tier::AcceptEdits),
        ("page.commit", Tier::AcceptEdits),
        ("page.dialog.accept", Tier::AcceptEdits),
        ("page.dialog.prompt", Tier::AcceptEdits),
        ("bookmarks.edit", Tier::AcceptEdits),
        ("clipboard.write", Tier::FullAccess),
        ("network.intercept", Tier::FullAccess),
        ("webmcp.call", Tier::FullAccess),
        ("cookies.change", Tier::FullAccess),
        ("storage.change", Tier::FullAccess),
        ("raw.execute", Tier::FullAccess),
        ("bookmarks.remove", Tier::FullAccess),
        ("history.delete", Tier::FullAccess),
        ("auth.fill", Tier::FullAccess),
        ("auth.basic", Tier::FullAccess),
    ];
    WRITES
        .iter()
        .filter(move |(_, needed)| *needed <= tier)
        .map(|(action, _)| *action)
}
#[cfg(test)]
#[path = "chrome_tests.rs"]
mod tests;
