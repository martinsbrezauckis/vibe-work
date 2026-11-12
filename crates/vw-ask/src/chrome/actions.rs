//! The single action-to-permission table for the owned helper. Depends on vw-ask;
//! used by the checked gateway and owner policy, so both enforce the same groups.
use super::{Group, Need, SecretRead, Tier};
use serde_json::{Value, json};

// action, helper capability, Vibe Work permission
const ACTIONS: &[(&str, &str, Need)] = &[
    (
        "recording.video.start",
        "capture.screenshot",
        Need::Group(Group::Read),
    ),
    (
        "recording.video.stop",
        "capture.screenshot",
        Need::Group(Group::Read),
    ),
    ("capture.page", "page.read", Need::Group(Group::Read)),
    ("capture.media", "page.read", Need::Group(Group::Read)),
    ("extensions.list", "page.read", Need::Group(Group::Read)),
    ("extensions.status", "page.read", Need::Group(Group::Read)),
    ("notifications.list", "page.read", Need::Group(Group::Read)),
    (
        "notifications.create",
        "page.commit",
        Need::Group(Group::Send),
    ),
    (
        "notifications.clear",
        "page.commit",
        Need::Group(Group::Send),
    ),
    ("tabs.audio", "page.read", Need::Group(Group::Read)),
    ("tabs.mute", "page.read", Need::Group(Group::Read)),
    ("media.read", "page.read", Need::Group(Group::Read)),
    ("media.play", "page.read", Need::Group(Group::Read)),
    ("media.pause", "page.read", Need::Group(Group::Read)),
    ("media.seek", "page.read", Need::Group(Group::Read)),
    ("media.mute", "page.read", Need::Group(Group::Read)),
    ("media.transcript", "page.read", Need::Group(Group::Read)),
    (
        "windows.list",
        "tabs.inspect_other",
        Need::Group(Group::Read),
    ),
    (
        "clipboard.read_rich",
        "clipboard.read",
        Need::SecretRead(SecretRead::Clipboard),
    ),
    (
        "clipboard.write_rich",
        "clipboard.write",
        // Its receipt reads actual clipboard content, including content replaced by another owner.
        Need::SecretRead(SecretRead::Clipboard),
    ),
    (
        "clipboard.read",
        "clipboard.read",
        Need::SecretRead(SecretRead::Clipboard),
    ),
    (
        "clipboard.write",
        "clipboard.write",
        // Read-back can contain another clipboard owner’s replacement.
        Need::SecretRead(SecretRead::Clipboard),
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
    (
        "page.add_style",
        "raw.execute",
        Need::SecretRead(SecretRead::PageScript),
    ),
    ("console.clear", "page.read", Need::Group(Group::Read)),
    ("page.inspect", "page.read", Need::Group(Group::Read)),
    ("page.errors.clear", "page.read", Need::Group(Group::Read)),
    ("network.list", "page.read", Need::Group(Group::Read)),
    ("network.headers", "page.read", Need::Group(Group::Read)),
    (
        "network.body",
        "files.download",
        Need::SecretRead(SecretRead::ResponseBody),
    ),
    ("auth.fill", "auth.secrets", Need::Group(Group::SignIn)),
    ("tabs.list", "tabs.inspect_other", Need::Group(Group::Read)),
    ("tabs.pin", "tabs.create", Need::Group(Group::Navigate)),
    ("page.resolve_point", "page.read", Need::Group(Group::Read)),
    (
        "page.scroll_into_view",
        "page.interact",
        Need::Group(Group::Navigate),
    ),
    (
        "page.highlight",
        "page.interact",
        Need::Group(Group::Navigate),
    ),
    ("page.move", "page.interact", Need::Group(Group::Navigate)),
    ("page.wheel", "page.interact", Need::Group(Group::Navigate)),
    ("tabs.group", "tabs.create", Need::Group(Group::Forms)),
    (
        "tabs.group_update",
        "tabs.create",
        Need::Group(Group::Forms),
    ),
    ("tabs.ungroup", "tabs.create", Need::Group(Group::Navigate)),
    ("window.focus", "tabs.create", Need::Group(Group::Navigate)),
    (
        "window.close",
        "tabs.close_owned",
        Need::Group(Group::Navigate),
    ),
    (
        "tabs.context",
        "tabs.inspect_other",
        Need::Group(Group::Read),
    ),
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
    ("page.wait_for", "page.read", Need::Group(Group::Read)),
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
    ("page.insert_text", "page.edit", Need::Group(Group::Forms)),
    ("page.focus", "page.interact", Need::Group(Group::Navigate)),
    ("page.blur", "page.interact", Need::Group(Group::Navigate)),
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
    (
        "raw.execute",
        "raw.execute",
        Need::SecretRead(SecretRead::PageScript),
    ),
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

/// The known action names in the shared browser classification.
pub fn action_names() -> impl Iterator<Item = &'static str> {
    ACTIONS.iter().map(|(action, _, _)| *action)
}

/// Personal-browser actions routed through the helper and the shared classification.
pub fn external_action_names() -> impl Iterator<Item = &'static str> {
    action_names()
}

/// The action named by a browser tool call.
#[must_use]
pub fn action(tool: &str, args: &Value) -> String {
    if tool == "chrome_execute" {
        args["action"].as_str().unwrap_or_default().to_owned()
    } else {
        tool.to_owned()
    }
}

/// The authority needed by this action and its exact arguments.
#[must_use]
pub fn need(tool: &str, args: &Value) -> Need {
    if tool == "chrome_status" || tool == "chrome_session" {
        return Need::Nothing;
    }
    if action(tool, args) == "page.key" && clipboard_shortcut(&args["args"]).is_some() {
        return Need::SecretRead(SecretRead::Clipboard);
    }
    if action(tool, args) == "network.headers" && args["args"]["unredacted"] == true {
        return Need::SecretRead(SecretRead::Headers);
    }
    if matches!(
        action(tool, args).as_str(),
        "cookies.change" | "storage.change"
    ) && args["args"]["operation"] == "clear"
    {
        return Need::ClearSiteData;
    }
    ACTIONS
        .iter()
        .find(|(id, _, _)| *id == action(tool, args))
        .map_or(Need::Unknown, |(_, _, need)| *need)
}

/// Recognize a clipboard command from the checked browser arguments.
#[must_use]
pub fn clipboard_shortcut(args: &Value) -> Option<super::ClipboardShortcut> {
    let modifiers = args["modifiers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    super::clipboard_shortcut(args["key"].as_str().unwrap_or_default(), &modifiers)
}

/// The package tier keeps broad helper capabilities out of lower-mode leases.
fn package_required(tool: &str, args: &Value) -> Option<Tier> {
    if need(tool, args).full_access_only() {
        return Some(Tier::FullAccess);
    }
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
        Need::Group(_) | Need::SecretRead(_) | Need::ClearSiteData => Some(Tier::FullAccess),
        Need::Unknown => None,
    }
}

/// Wider modes include every ordinary read allowed in Ask; writes retain their package tier.
#[must_use]
pub fn required(tool: &str, args: &Value) -> Option<Tier> {
    let tier = package_required(tool, args)?;
    Some(if ask_required(tool, args) == Some(Tier::Ask) {
        tier.min(Tier::AcceptEdits)
    } else {
        tier
    })
}

/// Ask separates ordinary reads/navigation from write packages; secret reads retain Full access.
#[must_use]
pub fn ask_required(tool: &str, args: &Value) -> Option<Tier> {
    let tier = package_required(tool, args)?;
    if need(tool, args).full_access_only() {
        return Some(Tier::FullAccess);
    }
    if action(tool, args) == "tabs.emulate"
        && args["args"]["userAgent"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    {
        return Some(Tier::AcceptEdits);
    }
    if super::asks(
        &action(tool, args),
        args["args"]["key"].as_str(),
        args["args"].get("title").is_some(),
    ) {
        Some(tier.max(Tier::AcceptEdits))
    } else {
        Some(Tier::Ask)
    }
}

/// A tool grant cannot substitute for the chat's mode for Full-only operations.
///
/// # Errors
/// Returns the reason and the person's next step when this action needs a higher chat mode.
pub fn check_full(mode: Option<&str>, tool: &str, args: &Value) -> Result<(), String> {
    if action(tool, args) == "page.key" {
        super::keys::validate_args(&args["args"])?;
    }
    if mode.and_then(Tier::from_mode) != Some(Tier::FullAccess)
        && let Some(reason) = need(tool, args).full_access_refusal()
    {
        return Err(reason);
    }
    Ok(())
}

/// Approval options are cumulative too; Full-only reads never offer a card.
#[must_use]
pub fn approval_tier(mode: Option<&str>, tool: &str, args: &Value) -> Option<Tier> {
    let tier = mode.and_then(Tier::from_mode)?;
    if need(tool, args).full_access_only() {
        return None;
    }
    let required = if tier == Tier::Ask {
        ask_required(tool, args)?
    } else {
        required(tool, args)?
    };
    (required > tier).then_some(required)
}

/// Cumulative helper capabilities for one chat tier.
#[must_use]
pub fn capabilities(tier: Tier) -> Vec<&'static str> {
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

/// The helper capability required by one known action.
#[must_use]
pub fn capability(tool: &str, args: &Value) -> Option<&'static str> {
    ACTIONS
        .iter()
        .find(|(id, _, _)| *id == action(tool, args))
        .map(|(_, cap, _)| *cap)
}

/// Helper capabilities are admitted per call, never by a saved profile-wide level.
/// A helper policy whose grants are supplied by the checked app gateway.
#[must_use]
pub fn policy() -> Value {
    let policy: std::collections::BTreeMap<_, _> =
        ACTIONS.iter().map(|(_, cap, _)| (*cap, "ask")).collect();
    json!({"level":"browse", "policy":policy})
}
