//! The single action-to-permission table for the owned helper. Depends on vw-ask;
//! used by the checked gateway and owner policy, so both enforce the same groups.
use serde_json::{Value, json};
use vw_ask::{
    ChromeSettings,
    chrome::{Group, Need},
};

// action, helper capability, Vibe Work permission
const ACTIONS: &[(&str, &str, Need)] = &[
    ("page.read", "page.read", Need::Group(Group::Read)),
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
    (
        "scope.add_origin",
        "scope.add_origin",
        Need::Group(Group::Navigate),
    ),
    ("tabs.create", "tabs.create", Need::Group(Group::Navigate)),
    (
        "tabs.close_owned",
        "tabs.close_owned",
        Need::Group(Group::Navigate),
    ),
    // The gateway distinguishes select from basic input even though the helper groups them as page.interact.
    ("page.click", "page.interact", Need::Group(Group::Navigate)),
    ("page.type", "page.interact", Need::Group(Group::Navigate)),
    ("page.select", "page.interact", Need::Group(Group::Forms)),
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
    ("storage.read", "storage.read", Need::Group(Group::SiteData)),
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
    ("history.read", "history.read", Need::Group(Group::History)),
    (
        "history.delete",
        "history.delete",
        Need::Group(Group::History),
    ),
    ("tabs.close_user", "tabs.close_user", Need::CloseUserTabs),
];

pub(super) fn action(tool: &str, args: &Value) -> String {
    if tool == "chrome_execute" {
        args["action"].as_str().unwrap_or_default().to_owned()
    } else {
        tool.to_owned()
    }
}

pub(super) fn need(tool: &str, args: &Value) -> Need {
    if tool == "chrome_status" || tool == "chrome_task" {
        return Need::Nothing;
    }
    ACTIONS
        .iter()
        .find(|(id, _, _)| *id == action(tool, args))
        .map_or(Need::Unknown, |(_, _, need)| *need)
}

pub(super) fn capability(action: &str) -> Option<&'static str> {
    ACTIONS
        .iter()
        .find(|(id, _, _)| *id == action)
        .map(|(_, cap, _)| *cap)
}

pub(super) fn policy(settings: ChromeSettings, close_user_tabs: bool) -> Value {
    let mut policy = serde_json::Map::new();
    for (_, capability, need) in ACTIONS {
        let allow = match need {
            Need::Group(group) => settings.allows(*group),
            Need::CloseUserTabs => close_user_tabs,
            _ => false,
        };
        let permission = if allow { "allow" } else { "ask" };
        if policy.get(*capability).and_then(Value::as_str) != Some("allow") {
            policy.insert((*capability).to_owned(), json!(permission));
        }
    }
    // The helper advertises no saved-login action. Keep the capability explicitly unavailable.
    policy.insert("auth.secrets".to_owned(), json!("never"));
    json!({"level": settings.level, "policy": policy})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_actions_have_one_capability_and_unknown_actions_never_inherit_authority() {
        for (action, capability, _) in ACTIONS {
            assert_eq!(super::capability(action), Some(*capability));
            assert_ne!(
                need("chrome_execute", &json!({"action":action})),
                Need::Unknown
            );
        }
        assert_eq!(
            need("chrome_execute", &json!({"action":"new.action"})),
            Need::Unknown
        );
        let browse = policy(ChromeSettings::default(), false);
        assert_eq!(browse["policy"]["page.commit"], "ask");
        assert_eq!(browse["policy"]["page.read"], "allow");
        assert_eq!(browse["policy"]["page.interact"], "allow");
        assert_eq!(browse["policy"].as_object().unwrap().len(), 23);
    }
}
