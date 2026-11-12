//! Grouped `browser_act` parsing for developer reads and the everyday input actions; called by `act::parse`.
//! Each returns `None` for a name it does not own, so `parse` keeps its one ordered dispatch.
use super::Action;
use serde_json::Value;

/// Network, page-script, viewport and element-inspection actions; `None` for any other name.
pub(super) fn developer(args: &Value) -> Option<Result<Action, String>> {
    Some(match args["action"].as_str()? {
        "network.list" | "network.headers" => {
            crate::network::Request::parse(args).map(Action::Network)
        }
        "raw.execute" | "page.add_style" => {
            crate::page_scripts::Request::parse(args).map(Action::PageScript)
        }
        "tabs.emulate" => crate::viewport::Request::parse(args).map(Action::Viewport),
        "page.inspect" => crate::element_inspection::Request::parse(args).map(Action::Inspect),
        _ => return None,
    })
}

/// Point resolution, pane scrolling, bounded waits and checked focus/blur/insertion; `None` for any other call.
pub(super) fn input(args: &Value) -> Option<Result<Action, String>> {
    if args["action"] == "resolve_point" {
        return Some(super::coordinate(args, "x").and_then(|x| {
            Ok(Action::ResolvePoint {
                x,
                y: super::coordinate(args, "y")?,
            })
        }));
    }
    if args["action"] == "scroll_position"
        || args["action"] == "scroll"
            && (["x", "y"].iter().any(|k| args.get(k).is_some())
                || matches!(args["direction"].as_str(), Some("left" | "right"))
                || args["ref"].is_string()
                    && (args.get("direction").is_some() || args.get("amount").is_some()))
    {
        return Some(crate::scroll_motion::Request::parse(args).map(Action::ScrollMotion));
    }
    if args["action"] == "wait"
        && (args["load"].is_string()
            || [
                "selector",
                "state",
                "ms",
                "url",
                "networkIdle",
                "expression",
            ]
            .iter()
            .any(|key| args.get(key).is_some()))
    {
        return Some(crate::wait_condition::Request::parse(args).map(Action::WaitCondition));
    }
    if matches!(
        args["action"].as_str(),
        Some("focus" | "blur" | "insert_text" | "highlight")
    ) || args["action"] == "type" && args["clear"] == false
    {
        return Some(crate::control::Request::parse(args).map(Action::Control));
    }
    None
}
