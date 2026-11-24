//! Shape checks on Chrome tool arguments, before any session, consent or helper work.
use crate::chrome_control::{agent_browser::Browser, browser_text::Fixed};
use serde_json::Value;

/// Refuses reserved and internal argument fields, in `browser`'s words.
pub(in crate::chrome_control) fn validate_public_args(
    args: &Value,
    browser: Browser,
) -> Result<(), String> {
    if args.as_object().is_none_or(|args| {
        args.keys().any(|key| {
            key.starts_with('_')
                || key == "approvedCapabilities"
                || key == "clientSessionId"
                || key == "sessionFolder"
                || key == "loginValue"
                || key == "preflightOperationId"
        })
    }) {
        return Err(Fixed::InvalidArguments.text(browser));
    }
    Ok(())
}

/// Keeps the single-action/batch contract explicit while native discovery receives a flat object schema.
pub(crate) fn validate_execute_args(args: &Value, browser: Browser) -> Result<(), String> {
    validate_public_args(args, browser)?;
    crate::chrome_control::output_file::validate(args)?;
    if args.get("steps").is_some() {
        return crate::chrome_control::batch::steps(args).map(|_| ());
    }
    if !args["action"].is_string() || !args["args"].is_object() {
        return Err("Not run: an action takes action and args, or use a batch of steps.".into());
    }
    Ok(())
}

pub(in crate::chrome_control) fn validate_session_args(
    args: &Value,
    browser: Browser,
) -> Result<(), String> {
    validate_public_args(args, browser)?;
    crate::chrome_control::profile_ids::validate_browser(args)?;
    if args.get("title").is_some() {
        return Err("Not run: Vibe Work supplies this chat's browser session title; omit title from the tool arguments.".into());
    }
    let field = match args["action"].as_str() {
        Some("open" | "close_browser") => "profileId",
        Some("get" | "cancel" | "stop") => "sessionId",
        _ => {
            return Err(
                "Not run: chrome_session requires action: open, get, cancel, stop or close_browser.".into(),
            );
        }
    };
    if args[field].as_str().is_none_or(|id| id.trim().is_empty()) {
        return Err(format!(
            "Not run: chrome_session requires {field} for this action."
        ));
    }
    Ok(())
}
