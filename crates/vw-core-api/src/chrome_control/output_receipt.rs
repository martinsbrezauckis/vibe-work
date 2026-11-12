//! Compact verified receipts before model budgets; remove older upload receipts' unrelated text-field value.
//! Other page data is not traversed. Called by the final Chrome output adapter.
use serde_json::{Value, json};

fn upload_outcome(body: &mut Value) -> bool {
    if body["action"] != "files.upload" || body["status"] != "completed" {
        return false;
    }
    let data = &mut body["untrustedPageContent"]["data"];
    let (status, reason) = if data
        .get("currentFiles")
        .is_some_and(|files| files != &data["files"])
    {
        (
            "failed",
            "The page changed or cleared the file selection after attachment. Read the file field before deciding whether to attach the files again.",
        )
    } else if data["fieldReplaced"] == true
        || data["resultObserved"] == false
        || !data["files"].is_array()
    {
        (
            "unknown",
            "The files were dispatched, but the final file selection could not be read back. Inspect the page's upload result before deciding whether to repeat the attachment.",
        )
    } else {
        data["verification"] = json!("file_list_read_back");
        return false;
    };
    data["verification"] = json!(status);
    data["instruction"] = json!(reason);
    body["status"] = json!(status);
    if body["untrustedPageContent"].get("status").is_some() {
        body["untrustedPageContent"]["status"] = json!(status);
    }
    body["error"] = json!({"code":"UPLOAD_NOT_CONFIRMED","message":reason});
    true
}

pub(super) fn compact(result: &mut Value) {
    if upload_outcome(&mut result["structuredContent"]) {
        result["isError"] = json!(true);
    }
    let body = &mut result["structuredContent"];
    if !body["operationId"].is_string() || !body["action"].is_string() {
        return;
    }
    let upload = body["action"] == "files.upload";
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
    // A file input has a FileList, never an editable text value. Older adapters could fall back to a focused editor.
    if upload && let Some(data) = payload.get_mut("data").and_then(Value::as_object_mut) {
        data.remove("value");
    }
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
