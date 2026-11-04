//! Common context for oversized batch receipts; the final output guard calls this before trimming payloads.
//! Every step keeps its outcome, operation and untrusted boundary; equal envelope fields have explicit defaults.
use serde_json::{Map, Value};

pub(super) fn share_context(body: &mut Value, limit_notes: [&str; 2]) {
    let Some(steps) = body.get_mut("steps").and_then(Value::as_array_mut) else {
        return;
    };
    for step in steps.iter_mut() {
        let result = &mut step["result"];
        // The final batch guard states the same cap once for the whole result.
        if result["outputLimit"]
            .as_str()
            .is_some_and(|note| limit_notes.contains(&note))
        {
            result
                .as_object_mut()
                .map(|result| result.remove("outputLimit"));
        }
        let Some(fields) = result.as_object().cloned() else {
            continue;
        };
        if let Some(payload) = result
            .get_mut("untrustedPageContent")
            .and_then(Value::as_object_mut)
        {
            for (key, value) in &fields {
                if key != "untrustedPageContent" && payload.get(key) == Some(value) {
                    payload.remove(key);
                }
            }
        }
    }
    let mut defaults = Map::new();
    for key in [
        "sessionId",
        "profileId",
        "policyRevision",
        "leaseEpoch",
        "note",
        "promptGuard",
    ] {
        let Some(value) = steps.first().and_then(|s| s["result"].get(key)).cloned() else {
            continue;
        };
        if steps.iter().all(|s| s["result"].get(key) == Some(&value)) {
            defaults.insert(key.into(), value);
            for step in steps.iter_mut() {
                if let Some(result) = step["result"].as_object_mut() {
                    result.remove(key);
                }
            }
        }
    }
    if !defaults.is_empty() {
        body["receiptDefaults"] = Value::Object(defaults);
    }
}
