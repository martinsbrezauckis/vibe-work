//! Compact discovery after live support and chat-mode checks. Called by the gateway after page marking.
//! Schema JSON stays lossless and compact when a code-mode program is re-indented; shared status facts occur once.
use crate::core::Shared;
use serde_json::{Value, json};

/// Invalid discovery arguments need no connection or helper status: only the app's executable action names.
pub(super) fn unknown_action(action: &str) -> Option<Value> {
    if super::mapping::action_names().any(|known| known == action) {
        return None;
    }
    let mut groups = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for action in super::mapping::action_names() {
        if let Some((group, name)) = action.split_once('.') {
            groups.entry(group).or_default().push(name);
        }
    }
    let lines: Vec<_> = groups
        .into_iter()
        .map(|(group, mut names)| {
            names.sort_unstable();
            format!("{group}: {}", names.join(","))
        })
        .collect();
    let message = format!(
        "Unknown action. Valid actions (group.name):\n{}\nUse group.name with chrome_execute; tool names such as chrome_session are not actions; call that tool directly.",
        lines.join("\n")
    );
    Some(super::owner::failure(&message, "UNKNOWN_ACTION"))
}

pub(super) fn compact(shared: &Shared, chat: &str, action: Option<&str>, result: &mut Value) {
    let body = &mut result["structuredContent"];
    if body.get("untrustedPageContent").is_none() {
        return;
    }
    if let Some(action) = action {
        action_details(shared, chat, action, body);
        super::output::sync_text(result);
        return;
    }
    if super::output::fits(body, 6000) {
        return;
    }
    let status = &mut body["untrustedPageContent"];
    share_session_fields(status);
    if let Some(status) = status.as_object_mut() {
        status.remove("addon");
    }
    // The selected browser's state is already in the browser-labelled add-on list.
    if let Some(addons) = body["addons"].as_array_mut() {
        for addon in addons {
            if addon["label"] == addon["state"] {
                addon.as_object_mut().map(|a| a.remove("label"));
            }
        }
    }
    let status = &mut body["untrustedPageContent"];
    share_feature_lists(status);
    if let Some(rows) = status["capabilities"]["permissions"].as_array() {
        let exceptions: Vec<_> = rows
            .iter()
            .filter(|r| r["available"] != true)
            .cloned()
            .collect();
        status["capabilities"]["permissions"] = json!({"total":rows.len(),"defaultAvailable":true,
            "available":rows.iter().filter(|r| r["available"] == true).count(),"exceptions":exceptions});
    }
    if let Some(rows) = status["capabilities"]["actions"].as_array_mut() {
        for row in rows {
            if row["action"] == row["capability"] {
                row.as_object_mut().map(|r| r.remove("capability"));
            }
            if row["allowedInThisMode"] == true
                || row.get("needs").is_some()
                || row["approvalRequired"] == true
            {
                row.as_object_mut().map(|r| r.remove("allowedInThisMode"));
            }
        }
        status["capabilities"]["defaultAllowedInThisMode"] = json!(true);
    }
    // Action ids are keys, with the common fields declared once. Keep exceptional permissions explicit.
    if let Some(rows) = status["capabilities"]["actions"].as_array() {
        let actions: serde_json::Map<String, Value> = rows
            .iter()
            .filter_map(|row| {
                let mut fields = row.as_object()?.clone();
                let id = fields.remove("action")?.as_str()?.to_owned();
                Some((id, Value::Object(fields)))
            })
            .collect();
        status["capabilities"]["actions"] = json!(actions);
        status["capabilities"]["defaultCapability"] = json!("action ID");
    }
    // Repeated mode labels are represented by a table too, including conditional typing-key authority.
    let mut needs = Vec::<Value>::new();
    if let Some(actions) = status["capabilities"]["actions"].as_object_mut() {
        for row in actions.values_mut() {
            for key in ["needs", "typingKeysNeed"] {
                if let Some(label) = row.get(key).cloned() {
                    let index = needs.iter().position(|v| v == &label).unwrap_or_else(|| {
                        needs.push(label);
                        needs.len() - 1
                    });
                    row[key] = json!(index);
                }
            }
        }
    }
    if !needs.is_empty() {
        status["capabilities"]["modes"] = json!(needs);
    }
    if let Some(status) = status.as_object_mut() {
        status.remove("owner");
    }
    status["capabilities"]["availability"] = json!(
        "needs/typingKeysNeed index modes; needs means unavailable in this mode. Profile feature lists define support. Use chrome_status({action}) for schema."
    );
    status["capabilities"]["execution"] =
        json!("Receipts report outcomes. Never replay an unknown outcome.");
    status["capabilities"]["savedLogin"] = json!(
        "Use auth.fill with a named vault login; vault consent applies. No Chrome password extraction."
    );
    if !super::output::fits(body, 6000) {
        let capabilities = &mut body["untrustedPageContent"]["capabilities"];
        share_capability_names(capabilities);
        capabilities["availability"] = json!(
            "capability indexes capabilityNames; needs/typingKeysNeed index modes; needs means unavailable in this mode. Profile feature lists define support. Use chrome_status({action}) for schema."
        );
    }
    super::output::sync_text(result);
}

// Chrome and Edge use the same permission names. Keep every action's binding, naming each permission once.
fn share_capability_names(capabilities: &mut Value) {
    let mut names = Vec::new();
    if let Some(actions) = capabilities["actions"].as_object_mut() {
        for row in actions.values_mut() {
            if let Some(name) = row.get("capability").cloned() {
                let index = names.iter().position(|v| v == &name).unwrap_or_else(|| {
                    names.push(name);
                    names.len() - 1
                });
                row["capability"] = json!(index);
            }
        }
    }
    capabilities["capabilityNames"] = json!(names);
}

fn share_session_fields(status: &mut Value) {
    let Some(rows) = status["sessions"].as_array_mut() else {
        return;
    };
    if rows.len() < 2 {
        return;
    }
    let mut defaults = serde_json::Map::new();
    for key in [
        "clientId",
        "clientName",
        "clientSessionId",
        "desiredState",
        "leaseEpoch",
        "policyRevision",
        "scope",
        "capture",
        "lastConfirmed",
        "groupTabs",
        "unresolved",
        "suspensionReason",
    ] {
        let Some(value) = rows[0].get(key).cloned() else {
            continue;
        };
        if rows.iter().all(|row| row.get(key) == Some(&value)) {
            defaults.insert(key.into(), value);
            for row in rows.iter_mut() {
                row.as_object_mut().map(|row| row.remove(key));
            }
        }
    }
    if !defaults.is_empty() {
        status["sessionDefaults"] = json!(defaults);
    }
}

fn action_details(shared: &Shared, chat: &str, action: &str, body: &mut Value) {
    let status = &mut body["untrustedPageContent"];
    let selected = shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(chat)
        .cloned();
    let profiles = status["profiles"].as_array();
    let connected = profiles.is_some_and(|rows| rows.iter().any(|p| p["connected"] == true));
    let supported = profiles.is_some_and(|rows| {
        rows.iter().any(|p| {
            p["connected"] == true
                && selected.as_deref().is_none_or(|id| {
                    p["profileId"]
                        .as_str()
                        .is_some_and(|p| super::profile_ids::same(p, id))
                })
                && p["features"]
                    .as_array()
                    .is_some_and(|features| features.iter().any(|f| f == action))
        })
    });
    let row = status["capabilities"]["actions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["action"] == action))
        .cloned();
    let mut row = row.unwrap_or_else(|| {
        json!({"action":action,"available":false,
        "reason":"Action unavailable; use chrome_status to find supported actions."})
    });
    if let Some(row) = row.as_object_mut() {
        row.remove("capability");
        row.insert("supported".into(), json!(supported));
        // Programs often return many independent lookups. A JSON string avoids Vibe expanding every
        // schema's nested constraints with indentation, without dropping descriptions or validation data.
        if let Some(schema) = row.remove("inputSchema") {
            row.insert("inputSchemaJson".into(), json!(schema.to_string()));
        }
    }
    *status = row;
    body.as_object_mut().map(|body| body.remove("addons"));
    body["connection"] = json!(if connected {
        "Connected."
    } else {
        "Disconnected; open the browser with its extension enabled."
    });
}

fn share_feature_lists(status: &mut Value) {
    let mut lists: Vec<Value> = Vec::new();
    if let Some(profiles) = status["profiles"].as_array_mut() {
        for profile in profiles {
            for key in ["extensionCapabilities", "features"] {
                if let Some(list) = profile.get(key).filter(|v| v.is_array()).cloned() {
                    let index = lists.iter().position(|v| v == &list).unwrap_or_else(|| {
                        lists.push(list);
                        lists.len() - 1
                    });
                    profile[key] = json!({"list":index});
                }
            }
        }
    }
    status["featureLists"] = json!(lists);
}
