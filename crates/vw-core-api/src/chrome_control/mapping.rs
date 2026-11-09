//! The single action-to-permission table for the owned helper. Depends on vw-ask;
//! used by the checked gateway and owner policy, so both enforce the same groups.
use serde_json::{Value, json};
use vw_ask::chrome::{Group, Need, SecretRead, Tier};

// action, helper capability, Vibe Work permission
const ACTIONS: &[(&str, &str, Need)] = &[
    (
        "clipboard.read",
        "clipboard.read",
        Need::SecretRead(SecretRead::Clipboard),
    ),
    (
        "clipboard.write",
        "clipboard.write",
        // Same tier as clipboard work on the desktop (a shell command runs without asking only at Full access).
        Need::Group(Group::SiteData),
    ),
    (
        "network.intercept",
        "network.intercept",
        Need::Group(Group::Scripts),
    ),
    ("tabs.emulate", "tabs.emulate", Need::Group(Group::Forms)),
    (
        "files.wait_download",
        "files.download",
        Need::Group(Group::Forms),
    ),
    ("auth.basic", "auth.secrets", Need::Group(Group::SignIn)),
    ("files.drop", "files.upload", Need::Group(Group::Forms)),
    ("webmcp.list", "raw.execute", Need::Group(Group::Scripts)),
    ("webmcp.call", "raw.execute", Need::Group(Group::Scripts)),
    ("window.resize", "tabs.create", Need::Group(Group::Navigate)),
    (
        "recording.start",
        "capture.screenshot",
        Need::Group(Group::Read),
    ),
    (
        "recording.stop",
        "capture.screenshot",
        Need::Group(Group::Read),
    ),
    (
        "page.triple_click",
        "page.interact",
        Need::Group(Group::Navigate),
    ),
    (
        "page.middle_click",
        "page.interact",
        Need::Group(Group::Navigate),
    ),
    ("page.check", "page.edit", Need::Group(Group::Forms)),
    ("page.uncheck", "page.edit", Need::Group(Group::Forms)),
    (
        "capture.zoom",
        "capture.screenshot",
        Need::Group(Group::Read),
    ),
    ("console.read", "page.read", Need::Group(Group::Read)),
    ("network.list", "page.read", Need::Group(Group::Read)),
    ("network.headers", "page.read", Need::Group(Group::Read)),
    (
        "network.body",
        "files.download",
        Need::SecretRead(SecretRead::ResponseBody),
    ),
    ("auth.fill", "auth.secrets", Need::Group(Group::SignIn)),
    ("tabs.list", "tabs.inspect_other", Need::Group(Group::Read)),
    ("page.back", "page.navigate", Need::Group(Group::Navigate)),
    (
        "page.forward",
        "page.navigate",
        Need::Group(Group::Navigate),
    ),
    ("page.reload", "page.navigate", Need::Group(Group::Navigate)),
    ("page.errors", "page.read", Need::Group(Group::Read)),
    (
        "files.save_pdf",
        "files.download",
        Need::Group(Group::Forms),
    ),
    (
        "cookies.read",
        "cookies.read",
        Need::SecretRead(SecretRead::Cookies),
    ),
    ("page.read", "page.read", Need::Group(Group::Read)),
    ("page.find", "page.read", Need::Group(Group::Read)),
    ("page.wait", "page.read", Need::Group(Group::Read)),
    (
        "capture.screenshot",
        "capture.screenshot",
        Need::Group(Group::Read),
    ),
    (
        "tabs.inspect_other",
        "tabs.inspect_other",
        Need::Group(Group::Read),
    ),
    (
        "page.navigate",
        "page.navigate",
        Need::Group(Group::Navigate),
    ),
    ("tabs.create", "tabs.create", Need::Group(Group::Navigate)),
    (
        "tabs.close_owned",
        "tabs.close_owned",
        Need::Group(Group::Forms),
    ),
    // Typing and choosing require interaction authority.
    ("page.click", "page.interact", Need::Group(Group::Navigate)),
    ("page.type", "page.edit", Need::Group(Group::Forms)),
    ("page.select", "page.edit", Need::Group(Group::Forms)),
    ("page.key", "page.interact", Need::Group(Group::Navigate)),
    ("page.scroll", "page.interact", Need::Group(Group::Navigate)),
    (
        "files.download",
        "files.download",
        Need::Group(Group::Forms),
    ),
    (
        "files.list_downloads",
        "files.download",
        Need::Group(Group::Forms),
    ),
    ("files.upload", "files.upload", Need::Group(Group::Forms)),
    ("page.commit", "page.commit", Need::Group(Group::Send)),
    (
        "cookies.metadata",
        "cookies.metadata",
        Need::Group(Group::SiteData),
    ),
    (
        "cookies.change",
        "cookies.change",
        Need::Group(Group::SiteData),
    ),
    (
        "storage.metadata",
        "storage.metadata",
        Need::Group(Group::SiteData),
    ),
    (
        "storage.read",
        "storage.read",
        Need::SecretRead(SecretRead::Storage),
    ),
    (
        "storage.change",
        "storage.change",
        Need::Group(Group::SiteData),
    ),
    ("raw.execute", "raw.execute", Need::Group(Group::Scripts)),
    (
        "bookmarks.read",
        "bookmarks.read",
        Need::Group(Group::Bookmarks),
    ),
    (
        "bookmarks.edit",
        "bookmarks.edit",
        Need::Group(Group::Bookmarks),
    ),
    (
        "bookmarks.remove",
        "bookmarks.remove",
        Need::Group(Group::RemoveData),
    ),
    ("history.read", "history.read", Need::Group(Group::History)),
    (
        "history.delete",
        "history.delete",
        Need::Group(Group::RemoveData),
    ),
    (
        "tabs.close_user",
        "tabs.close_user",
        Need::Group(Group::Forms),
    ),
    ("page.hover", "page.interact", Need::Group(Group::Navigate)),
    (
        "page.double_click",
        "page.interact",
        Need::Group(Group::Navigate),
    ),
    (
        "page.right_click",
        "page.interact",
        Need::Group(Group::Navigate),
    ),
    ("page.drag", "page.dragdrop", Need::Group(Group::Forms)),
    ("page.dialogs", "page.read", Need::Group(Group::Read)),
    (
        "page.dialog.dismiss",
        "page.interact",
        Need::Group(Group::Navigate),
    ),
    (
        "page.dialog.accept",
        "page.commit",
        Need::Group(Group::Send),
    ),
    (
        "page.dialog.prompt",
        "page.prompt",
        Need::Group(Group::Forms),
    ),
];

pub(super) fn action_names() -> impl Iterator<Item = &'static str> {
    ACTIONS.iter().map(|(action, _, _)| *action)
}

pub(super) fn action(tool: &str, args: &Value) -> String {
    if tool == "chrome_execute" {
        args["action"].as_str().unwrap_or_default().to_owned()
    } else {
        tool.to_owned()
    }
}

pub(super) fn need(tool: &str, args: &Value) -> Need {
    if tool == "chrome_status" || tool == "chrome_session" {
        return Need::Nothing;
    }
    if action(tool, args) == "network.headers" && args["args"]["unredacted"] == true {
        return Need::SecretRead(SecretRead::Headers);
    }
    ACTIONS
        .iter()
        .find(|(id, _, _)| *id == action(tool, args))
        .map_or(Need::Unknown, |(_, _, need)| *need)
}

/// The package tier keeps broad helper capabilities out of lower-mode leases.
fn package_required(tool: &str, args: &Value) -> Option<Tier> {
    if action(tool, args) == "page.key" {
        let key = args["args"]["key"].as_str().unwrap_or_default();
        if !matches!(
            key,
            "Tab"
                | "Shift+Tab"
                | "Escape"
                | "Enter"
                | "Space"
                | " "
                | "ArrowUp"
                | "ArrowDown"
                | "ArrowLeft"
                | "ArrowRight"
                | "Home"
                | "End"
                | "PageUp"
                | "PageDown"
        ) {
            return Some(Tier::AcceptEdits);
        }
    }
    match need(tool, args) {
        Need::Nothing | Need::Group(Group::Read | Group::Navigate) => Some(Tier::Ask),
        Need::Group(Group::Forms | Group::Send | Group::Bookmarks | Group::History) => {
            Some(Tier::AcceptEdits)
        }
        Need::Group(_) | Need::SecretRead(_) => Some(Tier::FullAccess),
        Need::Unknown => None,
    }
}

/// Wider modes include every ordinary read allowed in Ask; writes retain their package tier.
pub(super) fn required(tool: &str, args: &Value) -> Option<Tier> {
    let tier = package_required(tool, args)?;
    Some(if ask_required(tool, args) == Some(Tier::Ask) {
        tier.min(Tier::AcceptEdits)
    } else {
        tier
    })
}

/// Ask separates ordinary reads/navigation from write packages; secret reads retain Full access.
pub(crate) fn ask_required(tool: &str, args: &Value) -> Option<Tier> {
    let tier = package_required(tool, args)?;
    if matches!(need(tool, args), Need::SecretRead(_)) {
        return Some(Tier::FullAccess);
    }
    if action(tool, args) == "tabs.emulate"
        && args["args"]["userAgent"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    {
        return Some(Tier::AcceptEdits);
    }
    if vw_ask::chrome::asks(&action(tool, args), args["args"]["key"].as_str()) {
        Some(tier.max(Tier::AcceptEdits))
    } else {
        Some(Tier::Ask)
    }
}

/// A tool grant cannot substitute for the chat's mode when reading secret values.
pub(super) fn check_full(mode: Option<&str>, tool: &str, args: &Value) -> Result<(), String> {
    if mode.and_then(Tier::from_mode) != Some(Tier::FullAccess)
        && let Need::SecretRead(secret) = need(tool, args)
    {
        return Err(secret.refusal());
    }
    Ok(())
}

/// Approval options are cumulative too; Full-only reads never offer a card.
pub(super) fn approval_tier(mode: Option<&str>, tool: &str, args: &Value) -> Option<Tier> {
    let tier = mode.and_then(Tier::from_mode)?;
    if matches!(need(tool, args), Need::SecretRead(_)) {
        return None;
    }
    let required = if tier == Tier::Ask {
        ask_required(tool, args)?
    } else {
        required(tool, args)?
    };
    (required > tier).then_some(required)
}

pub(super) fn capabilities(tier: Tier) -> Vec<&'static str> {
    let mut caps = std::collections::BTreeSet::new();
    for (action, cap, _) in ACTIONS {
        if package_required("chrome_execute", &json!({"action":action,"args":{}}))
            .is_some_and(|need| need <= tier)
        {
            caps.insert(*cap);
        }
    }
    caps.into_iter().collect()
}

pub(super) fn capability(tool: &str, args: &Value) -> Option<&'static str> {
    ACTIONS
        .iter()
        .find(|(id, _, _)| *id == action(tool, args))
        .map(|(_, cap, _)| *cap)
}

/// Helper capabilities are admitted per call, never by a saved profile-wide level.
pub(super) fn policy() -> Value {
    let policy: std::collections::BTreeMap<_, _> =
        ACTIONS.iter().map(|(_, cap, _)| (*cap, "ask")).collect();
    json!({"level":"browse", "policy":policy})
}

#[cfg(test)]
mod tier_tests {
    use super::*;
    #[test]
    fn actions_match_the_owner_tiers() {
        for action in [
            "page.read",
            "page.click",
            "page.hover",
            "page.errors",
            "recording.start",
            "recording.stop",
        ] {
            assert_eq!(
                required("chrome_execute", &json!({"action":action})),
                Some(Tier::Ask),
                "{action}"
            );
        }
        for action in [
            "page.type",
            "page.select",
            "tabs.close_user",
            "tabs.close_owned",
            "files.upload",
            "page.commit",
            "webmcp.list",
            "cookies.metadata",
            "storage.metadata",
        ] {
            assert_eq!(
                required("chrome_execute", &json!({"action":action})),
                Some(Tier::AcceptEdits),
                "{action}"
            );
        }
        for action in [
            "raw.execute",
            "webmcp.call",
            "auth.fill",
            "auth.basic",
            "cookies.read",
            "history.delete",
        ] {
            assert_eq!(
                required("chrome_execute", &json!({"action":action})),
                Some(Tier::FullAccess),
                "{action}"
            );
        }
        assert_eq!(
            required("chrome_execute", &json!({"action":"future.action"})),
            None
        );
        assert!(!capabilities(Tier::Ask).contains(&"page.commit"));
        assert!(capabilities(Tier::AcceptEdits).contains(&"page.commit"));
    }
    #[test]
    fn diagnostic_reads_and_unredacted_headers_have_distinct_tiers() {
        for (action, args, tier) in [
            ("console.read", json!({}), Tier::Ask),
            ("network.list", json!({}), Tier::Ask),
            ("network.headers", json!({}), Tier::Ask),
            (
                "network.headers",
                json!({"unredacted":true}),
                Tier::FullAccess,
            ),
            ("network.body", json!({}), Tier::FullAccess),
        ] {
            assert_eq!(
                required("chrome_execute", &json!({"action":action,"args":args})),
                Some(tier)
            );
        }
    }
    #[test]
    fn typing_keys_are_interactions() {
        for key in ["a", "Control+V", "Backspace", "Delete", "Shift+A"] {
            assert_eq!(
                required(
                    "chrome_execute",
                    &json!({"action":"page.key","args":{"key":key}})
                ),
                Some(Tier::AcceptEdits)
            );
        }
        assert_eq!(
            required(
                "chrome_execute",
                &json!({"action":"page.key","args":{"key":"Tab"}})
            ),
            Some(Tier::Ask)
        );
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    #[test]
    fn every_advertised_action_has_exactly_one_permission_row() {
        let tools = vw_agent_tools::chrome_tool_list();
        let tool = tools
            .iter()
            .find(|t| t["name"] == "chrome_execute")
            .unwrap();
        // Vibe caches tool schemas across chats; discover action names through live chrome_status.
        assert_eq!(
            tool.pointer("/inputSchema/properties/action/type"),
            Some(&json!("string"))
        );
        let mut names = std::collections::HashSet::new();
        for (action, _, _) in ACTIONS {
            assert!(names.insert(action), "duplicate {action}");
        }
        if let Ok(path) = std::env::var("VW_CHROME_CATALOG") {
            let catalog: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let rows = catalog["actions"].as_array().unwrap();
            // A pinned helper may predate optional app actions. Every action it advertises must
            // still have exactly the app's permission; the independent fixture pins the latest set.
            assert!(rows.len() <= ACTIONS.len());
            for row in rows {
                assert!(
                    ACTIONS
                        .iter()
                        .any(|(a, c, _)| row["id"] == *a && row["capability"] == *c),
                    "{row}"
                );
            }
        }
    }
}
