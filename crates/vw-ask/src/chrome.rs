//! Pure browser tiers derived from chat access modes. Used by the core's action map; no stored permissions.
/// The common browser action classification used by every gateway.
pub mod actions;
/// Page-only key and shortcut scope for every browser.
pub mod keys;

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
    /// Removing all cookies or a complete storage area requires the chat's Full access mode.
    ClearSiteData,
    /// An unrecognised action; it has no authority.
    Unknown,
}

impl Need {
    /// A tool approval cannot authorize these operations below the chat's Full access mode.
    #[must_use]
    pub fn full_access_only(self) -> bool {
        matches!(self, Self::SecretRead(_) | Self::ClearSiteData)
    }

    /// Explain a Full-only refusal and the person's next step, when this need has one.
    #[must_use]
    pub fn full_access_refusal(self) -> Option<String> {
        match self {
            Self::SecretRead(secret) => Some(secret.refusal()),
            Self::ClearSiteData => {
                let full = Tier::FullAccess.label();
                Some(format!(
                    "Not run: clearing this site's data needs {full}, because it can sign the user out and remove saved work; ask the user to switch this chat to {full} if the task needs it."
                ))
            }
            _ => None,
        }
    }
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
    /// Arbitrary page code can read site cookies and storage.
    PageScript,
}
impl SecretRead {
    /// A refusal explains the risk and tells the agent how the person can authorize the read.
    #[must_use]
    pub fn refusal(self) -> String {
        let (operation, reason) = match self {
            Self::Cookies => (
                "Reading this site's cookies",
                "they can sign in as the user",
            ),
            Self::Storage => (
                "Reading this site's stored values",
                "they can contain sign-in tokens and private data",
            ),
            Self::Clipboard => (
                "Reading clipboard content",
                "it can contain passwords and other private data",
            ),
            Self::Headers => (
                "Reading unredacted request or response headers",
                "they can contain cookies and sign-in credentials",
            ),
            Self::PageScript => (
                "Running arbitrary page JavaScript",
                "it can read site cookies, stored sign-in tokens and hidden page values",
            ),
            Self::ResponseBody => (
                "Reading a captured network response body",
                "it can contain sign-in tokens and private data that were never displayed on the page",
            ),
        };
        let full = Tier::FullAccess.label();
        format!(
            "Not run: {operation} needs {full}, because {reason}; ask the user to switch this chat to {full} if the task needs it."
        )
    }
}

/// A system clipboard keyboard command; ordinary typing and selection have no clipboard authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardShortcut {
    /// Copy the selected page content to the system clipboard.
    Copy,
    /// Cut selected content into the system clipboard.
    Cut,
    /// Paste system clipboard content into the page.
    Paste,
}
/// Recognize clipboard chords in either the key string or separate modifiers.
/// The shared browser classifier and native input guard use this same interpretation.
#[must_use]
pub fn clipboard_shortcut(key: &str, modifiers: &[&str]) -> Option<ClipboardShortcut> {
    let parts = key.split('+').map(str::trim).collect::<Vec<_>>();
    let key = parts.last()?.to_ascii_lowercase();
    let modifier = |names: &[&str]| {
        modifiers
            .iter()
            .copied()
            .chain(parts.iter().take(parts.len() - 1).copied())
            .any(|value| names.iter().any(|name| value.eq_ignore_ascii_case(name)))
    };
    let command = modifier(&["Meta", "Cmd", "Command", "Control", "Ctrl"]);
    let shift = modifier(&["Shift"]);
    match key.as_str() {
        "paste" => Some(ClipboardShortcut::Paste),
        "copy" => Some(ClipboardShortcut::Copy),
        "cut" => Some(ClipboardShortcut::Cut),
        "v" if command => Some(ClipboardShortcut::Paste),
        "x" if command => Some(ClipboardShortcut::Cut),
        "insert" if shift => Some(ClipboardShortcut::Paste),
        "c" | "insert" if command => Some(ClipboardShortcut::Copy),
        "delete" if shift => Some(ClipboardShortcut::Cut),
        _ => None,
    }
}

/// Whether a known browser operation enters data or sends information. Both gateways use this
/// rule for tool approval; the shared action table separately identifies reads requiring Full access.
#[must_use]
pub fn asks(action: &str, key: Option<&str>, has_title: bool) -> bool {
    match action {
        "page.key" => !navigation_key(key.unwrap_or_default()),
        "tabs.group" => has_title,
        "notifications.create"
        | "notifications.clear"
        | "clipboard.write"
        | "clipboard.write_rich"
        | "page.insert_text"
        | "network.intercept"
        | "auth.basic"
        | "auth.fill"
        | "files.drop"
        | "webmcp.call"
        | "page.type"
        | "files.upload"
        | "page.commit"
        | "cookies.change"
        | "storage.change"
        | "raw.execute"
        | "bookmarks.edit"
        | "tabs.group_update"
        | "bookmarks.remove"
        | "history.delete"
        | "page.drag"
        | "page.dialog.accept"
        | "page.dialog.prompt" => true,
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
        ("notifications.create", Tier::AcceptEdits),
        ("notifications.clear", Tier::AcceptEdits),
        ("tabs.emulate", Tier::AcceptEdits),
        ("page.type", Tier::AcceptEdits),
        ("page.insert_text", Tier::AcceptEdits),
        ("page.key", Tier::AcceptEdits),
        ("page.drag", Tier::AcceptEdits),
        ("files.upload", Tier::AcceptEdits),
        ("files.drop", Tier::AcceptEdits),
        ("page.commit", Tier::AcceptEdits),
        ("page.dialog.accept", Tier::AcceptEdits),
        ("page.dialog.prompt", Tier::AcceptEdits),
        ("bookmarks.edit", Tier::AcceptEdits),
        ("tabs.group", Tier::AcceptEdits),
        ("tabs.group_update", Tier::AcceptEdits),
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
