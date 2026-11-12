//! Live extension support for the optional Chrome actions. The helper's authenticated status is the source;
//! the app catalog supplies schemas. No version threshold or remembered connection can imply support.
use crate::core::Shared;
use serde_json::{Value, json};

pub(super) const ACTIONS: &[&str] = &[
    "clipboard.read",
    "clipboard.write",
    "network.intercept",
    "tabs.emulate",
    "files.wait_download",
];
pub(super) const PERMISSIONS: &[&str] = &[
    "clipboard.read",
    "clipboard.write",
    "network.intercept",
    "tabs.emulate",
];
pub(super) fn gated(action: &str) -> bool {
    matches!(
        action,
        "page.find"
            | "tabs.context"
            | "tabs.pin"
            | "window.focus"
            | "window.close"
            | "windows.list"
            | "tabs.group"
            | "tabs.group_update"
            | "tabs.ungroup"
            | "page.resolve_point"
            | "page.move"
            | "page.wheel"
            | "page.focus"
            | "page.blur"
            | "page.insert_text"
            | "page.scroll_into_view"
            | "page.highlight"
            | "page.wait_for"
            | "clipboard.read_rich"
            | "clipboard.write_rich"
    ) || ACTIONS.contains(&action)
}
pub(super) fn supports(profile: &Value, action: &str) -> bool {
    // The helper implements find for older extensions too; its negotiated feature list is authoritative.
    if matches!(
        action,
        "page.find"
            | "tabs.context"
            | "tabs.pin"
            | "window.focus"
            | "window.close"
            | "windows.list"
            | "tabs.group"
            | "tabs.group_update"
            | "tabs.ungroup"
            | "page.resolve_point"
            | "page.move"
            | "page.wheel"
            | "page.focus"
            | "page.blur"
            | "page.insert_text"
            | "page.scroll_into_view"
            | "page.highlight"
            | "page.wait_for"
            | "clipboard.read_rich"
            | "clipboard.write_rich"
    ) {
        return profile["connected"] == true
            && profile["features"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|id| id == action));
    }
    profile["connected"] == true
        && profile["extensionVersion"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        && ["extensionCapabilities", "features"].iter().all(|field| {
            profile[*field]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|id| id == action))
        })
}
pub(super) fn filter(result: &mut Value, selected_profile: Option<&str>) {
    let body = &mut result["structuredContent"];
    let profiles = body["profiles"].as_array().cloned().unwrap_or_default();
    let supported = |id: &Value| {
        id.as_str().is_some_and(|id| {
            !gated(id)
                || profiles.iter().any(|p| {
                    selected_profile.is_none_or(|selected| {
                        p["profileId"]
                            .as_str()
                            .is_some_and(|id| super::profile_ids::same(id, selected))
                    }) && supports(p, id)
                })
        })
    };
    if let Some(profiles) = body["profiles"].as_array_mut() {
        for profile in profiles {
            let snapshot = profile.clone();
            if let Some(features) = profile["features"].as_array_mut() {
                features.retain(|id| {
                    id.as_str()
                        .is_some_and(|id| !gated(id) || supports(&snapshot, id))
                });
            }
        }
    }
    if let Some(rows) = body["capabilities"]["actions"].as_array_mut() {
        rows.retain(|row| supported(&row["action"]));
        // Keep argument descriptions in one app catalog, tested against the helper's source catalog.
        if let Ok(catalog) = serde_json::from_str::<Value>(include_str!("action_catalog.json")) {
            for row in rows.iter_mut() {
                super::connected_schema::select(row, selected_profile);
                if row.get("inputSchema").is_some()
                    && let Some(source) = catalog["actions"]
                        .as_array()
                        .and_then(|rows| rows.iter().find(|source| source["id"] == row["action"]))
                {
                    // An older pinned helper must keep its own schema and descriptions. New optional
                    // arguments are advertised only once that helper accepts them; extension gating is its job.
                    let properties = &row["inputSchema"]["properties"];
                    if source["inputSchema"]["properties"]
                        .as_object()
                        .is_some_and(|fields| {
                            fields.keys().any(|key| properties.get(key).is_none())
                        })
                    {
                        continue;
                    }
                    row["description"] = source["description"].clone();
                    row["inputSchema"] = source["inputSchema"].clone();
                }
            }
        }
        for row in rows {
            super::connected_schema::filter(row, &profiles, selected_profile);
        }
    }
    if let Some(permissions) = body["capabilities"]["permissions"].as_array_mut() {
        permissions.retain(|row| supported(&row["id"]));
    }
}
/// Re-read the helper for each new action (and each batch step), including its exact session/profile binding.
pub(super) async fn require(shared: &Shared, args: &Value) -> Result<(), String> {
    let action = args["action"].as_str().unwrap_or_default();
    if !gated(action) {
        return Ok(());
    }
    let reply = shared
        .chrome_control
        .owner
        .call("chrome_status", json!({}))
        .await;
    let body = &reply.value()["structuredContent"];
    let profile_id = body["sessions"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["sessionId"] == args["sessionId"])
        })
        .and_then(|row| row["profileId"].as_str());
    if profile_id.is_some_and(|id| {
        body["profiles"].as_array().is_some_and(|rows| {
            rows.iter().any(|p| {
                p["profileId"]
                    .as_str()
                    .is_some_and(|profile| super::profile_ids::same(profile, id))
                    && supports(p, action)
            })
        })
    }) {
        Ok(())
    } else {
        Err("Not run: this chat’s connected extension does not report support for this action. Update or enable the extension, then use chrome_status to find supported actions.".into())
    }
}
