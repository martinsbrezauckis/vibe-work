//! Explicit permission methods. Depends on no runtime state; core and policy drift tests consult this table.
use crate::Surface;
use crate::chrome::{self, Group};
/// Presentation and identity constraints for one class of permission.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// Method or callback kind.
    pub kind: &'static str,
    /// Fixed value-free title.
    pub title: &'static str,
    /// OS dialog or existing chat card.
    pub surface: Surface,
    /// Only a native person may supply an answer.
    pub person_only: bool,
    /// The Settings control an "Always" answer writes (its Settings id, `ui/src/features/settings/catalog.ts`).
    /// "Always" is offered only for a row naming a control Vibe Work owns, that exists on its own in
    /// Settings and that the user can see and change there later.
    /// Only the Chrome Control card for closing a tab the user opened names one: "Close tabs you opened". No card
    /// raises the Chrome Control level (a grant never reaches further than what the user saw). The vault "Always
    /// for this command / site / key" choices are the vault's own lasting grants (listed and revoked under Settings >
    /// Project secrets).
    pub always_setting: Option<&'static str>,
}
/// Settings controls a policy row may name as its `always_setting`; the drift tests check every entry against the
/// Settings catalog.
const ALWAYS_SETTINGS: &[(&str, &str)] = &[(chrome::CLOSE_TABS_KIND, "browser-close-user-tabs")];
/// The Chrome Control cards (`chrome.rs`): one kind per group, closing a tab the user opened, and an unknown tool.
const CHROME_CARDS: &[&str] = &[
    Group::Read.ask_kind(),
    Group::Navigate.ask_kind(),
    Group::Forms.ask_kind(),
    Group::Send.ask_kind(),
    Group::SignIn.ask_kind(),
    Group::SiteData.ask_kind(),
    Group::Scripts.ask_kind(),
    Group::Bookmarks.ask_kind(),
    Group::History.ask_kind(),
    chrome::CLOSE_TABS_KIND,
    chrome::OTHER_KIND,
    chrome::SESSION_KIND,
];
const NATIVE: &[&str] = &[
    "worktrees/start",
    "agents/install",
    "agents/uninstall",
    "config/proxy/write",
    "connector_catalog/auth/request",
    "mcp/add",
    "mcp/login",
    "mcp_catalog/add",
    "mcp_catalog/login",
    "projectLinks/create",
    "projectLinks/inspectRoot",
    "projectLinks/link",
    "projectLinks/list",
    "projectLinks/picker/load",
    "projectLinks/picker/loadMore",
    "projectLinks/resolveRoot",
    "projectLinks/save",
    "projectLinks/unlink",
    "skills/convertLocal",
    "skills/import",
    "skills/setLatest",
    "skills/setVersion",
    "vibeCode/projects/cancel",
    "vibeCode/projects/create",
    "vibeCode/projects/loadMore",
    "vibeCode/projects/open",
    "vibeCode/projects/recover",
    "vibeCode/projects/select",
    "vibeCode/projects/unlink",
    "vibeCode/teleport/cancel",
    "vibeCode/teleport/push/respond",
    "vibeCode/teleport/start",
    "session/agent/update",
    "config/write",
    "session/start",
    "session/resume",
    "session/fork",
    "session/continue",
    "session/delete",
    "session/shellCommand",
    "workspace/git/worktrees/remove",
    "workspace/trust/decision",
    "vibe-work/gitGuard",
    "vibe-work/killGuard",
    "vibe-work/secretRun",
    "vibe-work/fillLogin",
    "vibe-work/browserCloseTab",
    "vibe-work/providerConsent",
    "vibe-work/gitPush",
    "vibe-work/gitBringIn",
    "vibe-work/gitPullRequest",
    "vibe-work/worktreeStart",
    "vw/marketplace/install",
    "vw/marketplace/remove",
    "vw/plugins/turnOn",
    "vw/plugins/remove",
    "vibe-work/mcpServerKey",
    "vibe-work/captureSecret",
    "vibe-work/replaceVaultSecret",
    "vibe-work/replaceVaultLoginPassword",
    "vibe-work/replaceModelKey",
    "vibe-work/useModelKey",
    "files/open",
    "processes/stop",
    "vibe-work/projectInstructions/delete",
    chrome::LOOSEN_KIND,
    chrome::RESTORE_KIND,
    "newChat/restoreFolder",
    "chrome/control/fill",
];
const VAULT: &[&str] = &[
    "vibe-work/secretRun",
    "vibe-work/fillLogin",
    "vibe-work/mcpServerKey",
    "vibe-work/captureSecret",
    "vibe-work/replaceVaultSecret",
    "vibe-work/replaceVaultLoginPassword",
    "vibe-work/replaceModelKey",
    "vibe-work/useModelKey",
    "chrome/control/fill",
];
/// Every `(policy key, Settings control id)` pair an "Always" answer may set (see [`Policy::always_setting`]).
pub fn always_settings() -> impl Iterator<Item = (&'static str, &'static str)> {
    ALWAYS_SETTINGS.iter().copied()
}
/// Every known policy key; unknown runtime methods still fail closed as native asks.
pub fn policies() -> impl Iterator<Item = &'static str> {
    NATIVE
        .iter()
        .copied()
        .chain([
            "approval",
            "user_input",
            "connector_auth",
            "protected_change",
        ])
        .chain(CHROME_CARDS.iter().copied())
}
/// Resolve a method's fixed presentation policy. None means the method needs an explicit policy row.
#[must_use]
pub fn policy(kind: &str) -> Option<Policy> {
    let key = policies().find(|k| *k == kind)?;
    let card = matches!(
        key,
        "approval" | "user_input" | "connector_auth" | "protected_change"
    ) || CHROME_CARDS.contains(&key);
    let title = match key {
        "approval" => "this tool call",
        "user_input" => "this question",
        "protected_change" => "these protected-folder changes",
        "vibe-work/gitGuard" => "this Git command",
        "vibe-work/killGuard" => "this process-stop command",
        "vibe-work/secretRun" => "this use of a secret",
        "vibe-work/fillLogin" => "this saved login",
        "vibe-work/browserCloseTab" => "closing this tab",
        "vibe-work/providerConsent" => "this account access",
        chrome::SESSION_KIND => "using your Chrome in this chat",
        chrome::RESTORE_KIND => "the saved Chrome Control level",
        // The canonical "New chat" name lives in the shared vocabulary.
        "newChat/restoreFolder" => {
            vw_vocabulary::text("askRestoreFolderSubject").unwrap_or("the saved folder")
        }
        chrome::LOOSEN_KIND => "this change to Chrome Control's permissions",
        _ if CHROME_CARDS.contains(&key) => "this action in your Chrome",
        _ => "this request",
    };
    Some(Policy {
        kind: key,
        title,
        surface: if card { Surface::Card } else { Surface::Native },
        person_only: VAULT.contains(&key),
        always_setting: ALWAYS_SETTINGS
            .iter()
            .find(|(kind, _)| *kind == key)
            .map(|(_, setting)| *setting),
    })
}
