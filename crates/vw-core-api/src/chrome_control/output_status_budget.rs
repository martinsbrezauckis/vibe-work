//! Explicit fallback for a discovery status whose full capability tables cannot fit the agent output budget.
//! Called by `output::guard_status`; profiles, versions and add-on facts remain available for a focused lookup.
use serde_json::{Value, json};

pub(super) fn omit_capability_tables(body: &mut Value) -> bool {
    let status = &mut body["untrustedPageContent"];
    let Some(fields) = status.as_object_mut() else {
        return false;
    };
    if fields.remove("featureLists").is_none() {
        return false;
    }
    if let Some(profiles) = status["profiles"].as_array_mut() {
        for profile in profiles {
            if let Some(fields) = profile.as_object_mut() {
                fields.remove("features");
                fields.remove("extensionCapabilities");
            }
        }
    }
    if let Some(defaults) = status["profileDefaults"].as_object_mut() {
        defaults.remove("features");
        defaults.remove("extensionCapabilities");
    }
    status["capabilities"] = json!({
        "availability":"Full capability lists were omitted because they exceeded the status budget. Use chrome_status({action, profileId}) for a specific action and connected profile before using it; omission does not imply support."
    });
    true
}
