//! Built-in browser action aliases for Chrome's permission table.
//! Site data, clipboard and vault sign-in use the shared classification in every mode.
use serde_json::{Value, json};
use vw_ask::chrome::Tier;

/// The checked activation driver's executable escalation, beyond the tool's action names.
pub(crate) const SEND_ACTION: &str = "page.commit";

/// The approval verb for a built-in action: the shared catalog's name, else the wire name itself, so actions the catalog
/// does not describe (for example site-secret reads) are still classified by the mapping table (CD-01) rather than dropped.
fn action(args: &Value) -> &str {
    let name = args["action"].as_str().unwrap_or_default();
    vw_agent_tools::browser_input::actions()
        .get(name)
        .map_or(name, String::as_str)
}

/// Derive the package from every advertised executable action's existing tier, including native actions.
/// Unknown tiers refuse the package instead of silently leaving an executable action out of its disclosure.
pub(crate) fn package_actions(tier: Tier) -> Option<Vec<&'static str>> {
    if vw_agent_tools::browser_input::actions().is_empty() {
        return None;
    }
    let mut actions = Vec::new();
    for (wire, description) in vw_agent_tools::browser_input::actions() {
        let needed = required(&json!({"action":wire}))?;
        if needed > Tier::Ask && needed <= tier {
            actions.push(description.as_str());
        }
    }
    // A checked click/choice can become a form submission at execution time.
    if tier >= Tier::AcceptEdits {
        actions.push(SEND_ACTION);
    }
    actions.sort_unstable();
    actions.dedup();
    Some(actions)
}

fn native_required(args: &Value) -> Option<Tier> {
    match args["action"].as_str().unwrap_or_default() {
        "select"
        | "check"
        | "uncheck"
        | "page.dialogs"
        | "window.open"
        | "window.close"
        | "window.focus"
        | "page.dialog.dismiss"
        | "files.list_downloads"
        | "files.wait_download"
        | "capture.screenshot"
        | "page.click"
        | "page.double_click"
        | "page.right_click"
        | "page.middle_click"
        | "page.triple_click" => Some(Tier::Ask),
        "page.dialog.accept" | "page.dialog.prompt" | "files.save_download" => {
            Some(Tier::AcceptEdits)
        }
        _ => None,
    }
}

pub(crate) fn required(args: &Value) -> Option<Tier> {
    native_required(args).or_else(|| {
        let action = action(args);
        super::mapping::required("chrome_execute", &json!({"action":action,"args":args}))
    })
}

pub(crate) fn accepts_edits(args: &Value) -> bool {
    required(args).is_some_and(|tier| tier <= Tier::AcceptEdits)
}

pub(crate) fn ask_required(args: &Value) -> Option<Tier> {
    native_required(args).or_else(|| {
        let action = action(args);
        super::mapping::ask_required("chrome_execute", &json!({"action":action,"args":args}))
    })
}

pub(crate) fn check_full(mode: Option<&str>, args: &Value) -> Result<(), String> {
    super::mapping::check_full(
        mode,
        "chrome_execute",
        &json!({"action":action(args),"args":args}),
    )
}

pub(crate) fn approval_tier(mode: Option<&str>, args: &Value) -> Option<Tier> {
    if let Some(required) = native_required(args) {
        return (required > mode.and_then(Tier::from_mode)?).then_some(required);
    }
    super::mapping::approval_tier(
        mode,
        "chrome_execute",
        &json!({"action":action(args),"args":args}),
    )
}
