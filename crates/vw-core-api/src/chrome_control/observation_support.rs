//! Checks optional arguments against the helper and exact connected session before dispatch; gateway-only.
//! Helper-side find and returned receipt cursors remain usable with legacy extensions.
use crate::core::Shared;
use serde_json::{Value, json};

pub(super) async fn require(shared: &Shared, request: &Value) -> Result<(), String> {
    let keys: &[&str] = match request["action"].as_str() {
        Some("page.key") => &["frameId"],
        Some("page.read") => &["includeText", "cursor", "scope", "includeBounds", "summary"],
        Some("page.find") => &["summary", "scope"],
        Some("page.click" | "page.type" | "page.select") => &["observation"],
        _ => return Ok(()),
    };
    let requested: Vec<_> = keys
        .iter()
        .filter(|key| request["args"].get(**key).is_some())
        .filter(|key| {
            !(**key == "cursor"
                && request["args"]["cursor"]
                    .as_str()
                    .is_some_and(|c| c.starts_with("h-")))
        })
        .collect();
    if requested.is_empty() {
        return Ok(());
    }
    let reply = shared
        .chrome_control
        .owner
        .call("chrome_status", json!({"action":request["action"]}))
        .await;
    let body = &reply.value()["structuredContent"];
    let profile = body["sessions"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["sessionId"] == request["sessionId"])
        })
        .and_then(|row| row["profileId"].as_str());
    let mut row = body["capabilities"]["actions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["action"] == request["action"]))
        .cloned()
        .unwrap_or(Value::Null);
    super::connected_schema::select(&mut row, profile);
    super::connected_schema::filter(
        &mut row,
        body["profiles"].as_array().map_or(&[], Vec::as_slice),
        profile,
    );
    if profile.is_some()
        && requested
            .iter()
            .all(|key| row["inputSchema"]["properties"].get(**key).is_some())
    {
        Ok(())
    } else {
        Err("Not run: this browser connection does not accept these observation arguments. Use chrome_status with this action for its supported schema.".into())
    }
}
