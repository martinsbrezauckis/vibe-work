//! Remove repeated host context after receipt verification, before model budgets. Page data is never traversed.
use serde_json::Value;

pub(super) fn compact(result: &mut Value) {
    let body = &mut result["structuredContent"];
    if !body["operationId"].is_string() || !body["action"].is_string() {
        return;
    }
    let duplicates: Vec<_> = [
        "operationId",
        "sessionId",
        "profileId",
        "status",
        "state",
        "action",
        "capability",
        "policyRevision",
        "leaseEpoch",
    ]
    .into_iter()
    .filter(|key| body.get(key).is_some() && body[*key] == body["untrustedPageContent"][*key])
    .collect();
    let Some(payload) = body["untrustedPageContent"].as_object_mut() else {
        return;
    };
    // These describe the connection, available through discovery. Revision/lease and outcome stay above.
    for key in [
        "ownerPublicKey",
        "ownerFingerprint",
        "policy",
        "capabilities",
        "permissions",
        "browser",
    ] {
        payload.remove(key);
    }
    for key in duplicates {
        payload.remove(key);
    }
    if payload.get("error").is_some_and(Value::is_null) {
        payload.remove("error");
    }
    super::output::sync_text(result);
}
