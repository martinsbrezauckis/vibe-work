//! Compact multi-action discovery using existing single lookups, so older helpers remain compatible.
//! Gateway calls this after public argument validation; every lookup retains profile and mode filtering.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) fn requested(args: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(actions) = args.get("actions") else {
        return Ok(None);
    };
    let invalid = || {
        "Not run: chrome_status takes action or 1..8 unique actions, with an optional profileId."
            .to_owned()
    };
    if args.as_object().is_none_or(|m| {
        m.keys()
            .any(|key| !["actions", "profileId"].contains(&key.as_str()))
    }) || args.get("profileId").is_some_and(|id| !id.is_string())
    {
        return Err(invalid());
    }
    let rows = actions
        .as_array()
        .filter(|r| !r.is_empty() && r.len() <= 8)
        .ok_or_else(invalid)?;
    let mut result = Vec::new();
    for row in rows {
        let action = row
            .as_str()
            .filter(|a| {
                a.len() <= 128 && super::mapping::external_action_names().any(|known| known == *a)
            })
            .ok_or_else(invalid)?;
        if result.iter().any(|a| a == action) {
            return Err(invalid());
        }
        result.push(action.to_owned());
    }
    Ok(Some(result))
}

fn compact_schema(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if fields.get("description").is_some_and(Value::is_string) {
                fields.remove("description");
            }
            for (key, value) in fields {
                // Literal values are validation data, even when a property is named description.
                if !matches!(key.as_str(), "const" | "enum" | "default" | "examples") {
                    compact_schema(value);
                }
            }
        }
        Value::Array(rows) => {
            for row in rows {
                compact_schema(row);
            }
        }
        _ => {}
    }
}

pub(super) async fn call(
    shared: &Arc<Shared>,
    chat: &str,
    actions: &[String],
    profile: Option<&str>,
) -> Result<Value, String> {
    let mut rows = Vec::new();
    let mut first = None;
    let mut more_options = None;
    for action in actions {
        let mut args = json!({"action":action});
        if let Some(profile) = profile {
            args["profileId"] = json!(profile);
        }
        let mut answer = super::gateway::call_selected(shared, chat, "chrome_status", args).await?;
        let result = &mut answer["vwMcpResult"];
        if result["isError"] == true || result["structuredContent"].get("error").is_some() {
            return Ok(answer);
        }
        super::output_status::compact(shared, chat, Some(action), result);
        let mut row = result["structuredContent"]["untrustedPageContent"].clone();
        if let Some(encoded) = row
            .as_object_mut()
            .and_then(|r| r.remove("inputSchemaJson"))
        {
            row["inputSchema"] = serde_json::from_str(encoded.as_str().unwrap_or_default())
                .map_err(|_| "Not run: invalid action schema JSON; retry chrome_status.")?;
        }
        row.as_object_mut().map(|row| row.remove("description"));
        if let Some(schema) = row.get_mut("inputSchema") {
            compact_schema(schema);
        }
        if let Some(value) = row
            .as_object_mut()
            .and_then(|row| row.remove("moreOptions"))
        {
            more_options = Some(value);
        }
        rows.push(row);
        if first.is_none() {
            first = Some(answer);
        }
    }
    let mut answer = first.ok_or("Not run: no action schemas were requested.")?;
    let result = &mut answer["vwMcpResult"];
    result["structuredContent"]["untrustedPageContent"] = json!({"actions":rows});
    if let Some(note) = more_options {
        result["structuredContent"]["untrustedPageContent"]["moreOptions"] = note;
    }
    let text = result["structuredContent"]["untrustedPageContent"].to_string();
    let verdict = vw_browser::guard::check([("Chrome discovery".to_owned(), text.as_str())]);
    result["structuredContent"]["promptGuard"] = vw_browser::observe::guard_json(&verdict);
    if !super::output::fits(&result["structuredContent"], 9000) {
        return Err("Not run: these schemas exceed one discovery result. Request fewer actions; no schemas were omitted.".into());
    }
    super::output::sync_text(result);
    super::output::guard_status(result);
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_preserves_literal_descriptions_inside_schema_constraints() {
        let literal = json!({"description":"required literal", "nested":{"description":"exact"}});
        let mut schema = json!({
            "description":"annotation",
            "properties":{"description":{"type":"string","description":"field annotation"}},
            "const":literal,"enum":[literal],"default":literal,"examples":[literal],
            "allOf":[{"description":"branch annotation","minProperties":1}]
        });
        compact_schema(&mut schema);
        assert_eq!(schema["const"], literal);
        assert_eq!(schema["enum"], json!([literal]));
        assert_eq!(schema["default"], literal);
        assert_eq!(schema["examples"], json!([literal]));
        assert_eq!(
            schema["properties"]["description"],
            json!({"type":"string"})
        );
        assert_eq!(schema["allOf"], json!([{"minProperties":1}]));
        assert!(schema.get("description").is_none());
    }
}
