//! Named page schemas supplied by the app only for a live negotiated primitive transport.
//! Gateway discovery uses these with the same shared classification as built-in actions.
use serde_json::{Value, json};
pub(super) const NAMES: &[&str] = &[
    "recording.video.start",
    "recording.video.stop",
    "tabs.audio",
    "tabs.mute",
    "media.read",
    "media.play",
    "media.pause",
    "media.seek",
    "media.mute",
    "media.transcript",
];
fn schema(action: &str) -> Value {
    if super::page_capture::NAMES.contains(&action) {
        return super::page_capture::schema(action);
    }
    if super::extension_state::NAMES.contains(&action) {
        return super::extension_state::schema(action);
    }
    let mut properties = json!({"tabId":{"type":"integer","minimum":0,"maximum":2_147_483_647}});
    let mut required = vec!["tabId"];
    if action.starts_with("media.") && action != "media.read" {
        properties["mediaId"] = json!({"type":"string","maxLength":64,"description":"Exact mediaId from media.read in this document."});
        required.push("mediaId");
    }
    if action == "media.seek" {
        properties["timeMs"] = json!({"type":"integer","minimum":0,"maximum":604_800_000});
        required.push("timeMs");
    }
    if matches!(action, "media.mute" | "tabs.mute") {
        properties["muted"] = json!({"type":"boolean"});
        required.push("muted");
    }
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
}

pub(super) fn decorate(result: &mut Value, requested: Option<&str>) {
    let body = &mut result["structuredContent"];
    let Some(profiles) = body["profiles"].as_array_mut() else {
        return;
    };
    let mut supported_names = std::collections::BTreeSet::new();
    for profile in profiles {
        if !super::page_plan::supported(profile) {
            continue;
        }
        let names: Vec<_> = NAMES
            .iter()
            .chain(super::extension_state::NAMES)
            .chain(super::page_capture::NAMES)
            .filter(|name| super::page_plan::supported_action(profile, name))
            .copied()
            .collect();
        supported_names.extend(names.iter().copied());
        if let Some(features) = profile["features"].as_array_mut() {
            for name in names {
                if !features.contains(&json!(name)) {
                    features.push(json!(name));
                }
            }
        }
    }
    if supported_names.is_empty() {
        return;
    }
    if let Some(actions) = body["capabilities"]["actions"].as_array_mut() {
        if requested.is_some_and(super::page_plan::handles) {
            actions.clear();
        }
        for name in NAMES
            .iter()
            .chain(super::extension_state::NAMES)
            .chain(super::page_capture::NAMES)
            .filter(|name| {
                supported_names.contains(**name) && requested.is_none_or(|r| r == **name)
            })
        {
            if actions.iter().any(|row| row["action"] == *name) {
                continue;
            }
            let mut row = json!({"action":name,"capability":super::mapping::capability("chrome_execute", &json!({"action":name}))});
            if requested.is_some() {
                row["inputSchema"] = schema(name);
                row["description"] = if name.starts_with("recording.video.") {
                    json!(
                        "Continuous video requires an offscreen encoder absent from this extension version. This action returns unavailable with the existing recording.start/stop action-screenshot GIF alternative."
                    )
                } else if super::page_capture::NAMES.contains(name) {
                    json!(
                        "Save passive HTML, packed HTML, Markdown, text or loaded images/fonts in this chat folder. Text is capped at one million characters; media at 5 MiB each and 16 MiB total. Receipts report truncation and every unavailable resource."
                    )
                } else if super::extension_state::NAMES.contains(name) {
                    json!(
                        "Read extension grants/inventory or create, list and clear this chat’s named browser notifications. Notification changes read back the actual ID; sending a notification asks in Ask."
                    )
                } else {
                    json!(
                        "Read or control main-document video/audio and return actual state. Handles come from media.read. Transcript returns loaded text-track cues; unavailable captions are reported explicitly."
                    )
                };
            }
            actions.push(row);
        }
    }
}
