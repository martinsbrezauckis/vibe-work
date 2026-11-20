//! Explicit permission methods. Depends on no runtime state; core and policy drift tests consult this table.
use crate::Surface;
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
const ALWAYS_SETTINGS: &[(&str, &str)] = &[];
const NATIVE: &[&str] = &[
    "worktrees/start",
    "agents/install",
    "agents/uninstall",
    "config/proxy/write",
    "connector_catalog/auth/request",
    "connector_catalog/connect",
    "mcp/add",
    "mcp/login",
    "mcp_catalog/add",
    "mcp_catalog/login",
    "projectLinks/inspectRoot",
    "projectLinks/list",
    "projectLinks/resolveRoot",
    "projectLinks/save",
    "projectLinks/unlink",
    "skills/convertLocal",
    "skills/import",
    "skills/setLatest",
    "skills/setVersion",
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
    "vibe-work/providerConsent",
    "vibe-work/gitPush",
    "vibe-work/developerPermissions",
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
    "newChat/restoreFolder",
    "chrome/control/fill",
    "chrome/control/pair",
    "vibe-work/localNetworkAccess",
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
/// Every known policy key; unknown runtime methods still fail closed as native asks.
pub fn policies() -> impl Iterator<Item = &'static str> {
    NATIVE.iter().copied().chain([
        "vault_save",
        "approval",
        "user_input",
        "connector_auth",
        "protected_change",
    ])
}
/// Resolve a method's fixed presentation policy. None means the method needs an explicit policy row.
#[must_use]
pub fn policy(kind: &str) -> Option<Policy> {
    let key = policies().find(|k| *k == kind)?;
    let card = matches!(
        key,
        "approval" | "user_input" | "connector_auth" | "protected_change" | "vault_save"
    );
    let title = match key {
        "approval" => "this tool call",
        "user_input" => "this question",
        "protected_change" => "these protected-folder changes",
        "vibe-work/gitGuard" => "this Git command",
        "vibe-work/killGuard" => "this process-stop command",
        "vibe-work/secretRun" => "this use of a secret",
        "vibe-work/fillLogin" => "this saved login",
        "vibe-work/providerConsent" => "this account access",
        "vibe-work/developerPermissions" => vw_vocabulary::texts::DEVELOPER_PERMISSIONS_NAME,
        "chrome/control/pair" => "this browser connection",
        "vibe-work/localNetworkAccess" => "this site's access to your local network",
        // The canonical "New chat" name lives in the shared vocabulary.
        "newChat/restoreFolder" => {
            vw_vocabulary::text("askRestoreFolderSubject").unwrap_or("the saved folder")
        }
        _ => "this request",
    };
    Some(Policy {
        kind: key,
        title,
        surface: if card { Surface::Card } else { Surface::Native },
        // The built-in browser's Local Network Access question grants a site lasting access: the person only.
        person_only: VAULT.contains(&key)
            || matches!(
                key,
                "vault_save"
                    | "chrome/control/pair"
                    | "vibe-work/localNetworkAccess"
                    | "vibe-work/developerPermissions"
            ),
        always_setting: ALWAYS_SETTINGS
            .iter()
            .find(|(kind, _)| *kind == key)
            .map(|(_, setting)| *setting),
    })
}

#[cfg(test)]
mod tests {
    // PREDICT: Developer permissions is a native, person-only decision with no lasting answer.
    #[test]
    fn developer_permissions_requires_the_person_in_the_native_window() {
        let policy =
            super::policy("vibe-work/developerPermissions").expect("explicit developer policy");
        assert_eq!(policy.surface, crate::Surface::Native);
        assert!(policy.person_only);
        assert_eq!(policy.always_setting, None);
    }

    #[test]
    fn retired_transfers_and_experimental_moves_have_no_permission_surface() {
        for method in [
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
            "session/move",
            "plugins/local/list",
            "plugins/local/setEnabled",
            "projectLinks/create",
            "projectLinks/link",
            "projectLinks/picker/load",
            "projectLinks/picker/loadMore",
        ] {
            assert!(super::policy(method).is_none(), "{method}");
        }
    }
}
