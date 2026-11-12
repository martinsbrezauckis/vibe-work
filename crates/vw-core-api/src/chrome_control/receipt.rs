//! Host browser identity shared by approval cards and tool receipts; reads the action's retained lease binding.
//! Gateway and output-cache boundaries call this before final budgeting; page fields never supply identity.
use crate::core::Shared;
use serde_json::Value;
use std::sync::PoisonError;

pub(super) fn profile(shared: &Shared, tool: &str, args: &Value) -> Option<String> {
    if matches!(tool, "chrome_session" | "chrome_task") && args["action"] == "open" {
        return args["profileId"].as_str().map(str::to_owned);
    }
    args["sessionId"]
        .as_str()
        .and_then(|id| bound_profile(shared, id))
}

fn bound_profile(shared: &Shared, id: &str) -> Option<String> {
    let profile = shared
        .chrome_control
        .work
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .profile_of(id)
        .map(str::to_owned);
    profile.or_else(|| super::profile_bindings::session_profile(&shared.chrome_control, id))
}

pub(super) fn stamp(shared: &Shared, tool: &str, args: &Value, result: &mut Value) {
    if !matches!(tool, "chrome_execute" | "chrome_session") {
        return;
    }
    let opened = (tool == "chrome_session" && args["action"] == "open")
        .then(|| {
            result["structuredContent"]["sessionId"]
                .as_str()
                .and_then(|id| bound_profile(shared, id))
        })
        .flatten();
    let Some(profile) = opened.or_else(|| profile(shared, tool, args)) else {
        return;
    };
    let Some(body) = result["structuredContent"].as_object_mut() else {
        return;
    };
    let name = shared
        .chrome_control
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .name(&profile);
    body.insert("profileId".into(), Value::String(profile));
    body.insert("profileName".into(), Value::String(name));
    super::output::sync_text(result);
}
