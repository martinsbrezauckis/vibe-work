//! Optional action arguments follow the connected session/profile, including older helpers' full catalogs.
//! Called by discovery and observation admission; never infers capabilities from a version string.
use serde_json::{Value, json};

const OPTIONS: &[(&str, &str, &str)] = &[
    ("page.read", "includeText", "page.read.includeText"),
    ("page.read", "cursor", "page.read.cursor"),
    ("page.read", "scope", "page.read.scope.viewport"),
    ("page.read", "includeBounds", "page.read.includeBounds"),
    ("page.read", "summary", "page.read.summary"),
    ("page.find", "summary", "page.read.summary"),
    ("page.find", "scope", "page.find.scope.viewport"),
    ("page.key", "frameId", "page.key.frameId"),
    ("", "observation", "page.observation.changes"),
];

pub(super) fn select(row: &mut Value, selected: Option<&str>) {
    if let Some(schema) = row["profileSchemas"].as_object().and_then(|schemas| {
        schemas
            .iter()
            .find(|(id, _)| selected.is_some_and(|s| super::profile_ids::same(id, s)))
            .map(|(_, schema)| schema.clone())
    }) {
        if schema != row["inputSchema"] {
            row.as_object_mut().map(|row| row.remove("moreOptions"));
        }
        row["inputSchema"] = schema;
    }
    if let Some(row) = row.as_object_mut() {
        row.remove("profileSchemas");
    }
}

pub(super) fn filter(row: &mut Value, profiles: &[Value], selected: Option<&str>) {
    let action = row["action"].as_str().unwrap_or_default().to_owned();
    let profiles: Vec<_> = profiles
        .iter()
        .filter(|p| {
            p["connected"] == true
                && selected.is_none_or(|s| {
                    p["profileId"]
                        .as_str()
                        .is_some_and(|id| super::profile_ids::same(id, s))
                })
                && p["features"]
                    .as_array()
                    .is_some_and(|f| f.iter().any(|f| f == &action))
        })
        .collect();
    let Some(fields) = row
        .get_mut("inputSchema")
        .and_then(|s| s.get_mut("properties"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let mut removed = false;
    for (for_action, key, capability) in OPTIONS {
        if (*for_action == action || for_action.is_empty())
            && (profiles.is_empty()
                || !profiles.iter().all(|p| {
                    p["extensionCapabilities"]
                        .as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == capability))
                }))
        {
            removed |= fields.remove(*key).is_some();
        }
    }
    let summary = fields.contains_key("summary");
    let scope = fields.contains_key("scope");
    if removed {
        row["moreOptions"] = json!(
            "Optional arguments need connected extension and helper support; use the schema shown."
        );
        let description = row["description"].as_str().unwrap_or_default().to_owned();
        if action == "page.read" && !summary && description.contains("summary:true") {
            row["description"] = json!(
                "Controls + preview. interactiveOnly:false: text/structure; options:true: options; ref: subtree. maxChars: first preview. Continue with returned args; reassemble JSON details."
            );
        }
        if action == "page.find"
            && ((!summary && description.contains("summary:true"))
                || (!scope && description.contains("scope:viewport")))
        {
            let mut text = String::from(
                "Find by label, role, text or placeholder with deterministic ranking (five matches by default). Tied matches are ambiguous; inspect them before choosing a ref. Continue with the returned cursor for remaining candidates.",
            );
            if summary {
                text.push_str(
                    " summary:true returns captured match counts and coverage without refs.",
                );
            }
            if scope {
                text.push_str(" scope:viewport limits matching to the viewport.");
            }
            row["description"] = json!(text);
        }
        if action == "page.key" {
            row["description"] = json!(
                "Send a key or key combination to this tab's focused element. Use Meta for macOS editing shortcuts, Control elsewhere. Enter in a POST form requires sending permission before input. Input needs the tab active in its window."
            );
        }
    }
}
