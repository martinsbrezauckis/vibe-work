//! Shared crowded discovery representation: profile defaults and omitted unknown versions keep all browsers within budget.
//! Called by `output_status` after permissions are classified; known versions, instructions and distinct profile facts remain exact.
use serde_json::{Value, json};
/// Additional browser facts share repeated profile metadata only when the optional build exceeds its budget.
/// The raw helper state remains authoritative; this changes only the model-facing status representation.
pub(super) fn compact(body: &mut Value) {
    if super::super::output::fits(body, 6000) {
        return;
    }
    if let Some(addons) = body["addons"].as_array_mut() {
        for addon in addons {
            omit_unknown_version(addon);
            if let Some(sharing) = addon.get_mut("sharing") {
                omit_unknown_version(sharing);
            }
        }
    }
    let status = &mut body["untrustedPageContent"];
    let Some(profiles) = status["profiles"]
        .as_array_mut()
        .filter(|rows| rows.len() > 1)
    else {
        return;
    };
    let mut defaults = serde_json::Map::new();
    for key in [
        "browser",
        "connected",
        "browserVersion",
        "extensionVersion",
        "route",
        "features",
        "extensionCapabilities",
    ] {
        if let Some(value) = profiles[0].get(key).cloned()
            && profiles
                .iter()
                .all(|profile| profile.get(key) == Some(&value))
        {
            defaults.insert(key.into(), value);
            for profile in profiles.iter_mut() {
                profile.as_object_mut().map(|fields| fields.remove(key));
            }
        }
    }
    if defaults.is_empty() {
        return;
    }
    status["profileDefaults"] = Value::Object(defaults);
    if let Some(instruction) = status["capabilities"]["availability"].as_str() {
        status["capabilities"]["availability"] =
            json!(format!("{instruction} Profiles inherit profileDefaults."));
    }
    // A missing store verdict is already unknown; do not repeat an explicit null in this crowded overview.
    if status
        .get("chromeExtensionStore")
        .is_some_and(Value::is_null)
    {
        status
            .as_object_mut()
            .map(|fields| fields.remove("chromeExtensionStore"));
    }
}

fn omit_unknown_version(value: &mut Value) {
    if value.get("extensionVersion").is_some_and(Value::is_null) {
        value
            .as_object_mut()
            .map(|fields| fields.remove("extensionVersion"));
    }
}
