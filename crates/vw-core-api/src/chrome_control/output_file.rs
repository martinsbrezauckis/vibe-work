//! Explicit exports of one Chrome read, including its untrusted boundary and continuation; gateway/cache only.
//! Uses the authenticated chat folder and a newly created leaf through an open directory, never a shell or spill path.
use crate::core::Shared;
use cap_std::fs::{Dir, OpenOptions};
use serde_json::{Value, json};
use std::io::Write;

pub(super) fn requested(request: &Value) -> bool {
    request["saveTo"] == "file" || crate::browser_data::requested(request)
}

pub(super) fn validate(request: &Value) -> Result<(), String> {
    if let Some(option) = request.get("saveTo")
        && (!matches!(option.as_str(), Some("inline" | "file" | "csv" | "xlsx"))
            || request["action"] != "page.read"
            || !request["args"].is_object()
            || request.get("steps").is_some())
    {
        return Err("Not run: saveTo takes inline, file, csv or xlsx for a single page.read. Omit it for other actions or batches.".into());
    }
    if crate::browser_data::requested(request) {
        crate::browser_data::options(request, true)?;
    } else if ["extract", "appendFrom", "deduplicate"]
        .iter()
        .any(|key| request.get(*key).is_some())
    {
        return Err(
            "Extraction options need saveTo:csv or xlsx; choose the export format or omit them."
                .into(),
        );
    }
    Ok(())
}

pub(super) fn folder(shared: &Shared, chat: &str) -> Result<Dir, String> {
    crate::browser_data::folder(shared, chat)
}

pub(super) fn save(
    shared: &Shared,
    chat: &str,
    request: &Value,
    body: &Value,
) -> Result<Value, String> {
    if body["status"] != "completed"
        || !body["error"].is_null()
        || !body["untrustedPageContent"]["error"].is_null()
    {
        return Err("Not saved: this read failed or has no completed outcome; omit saveTo to read its diagnostic inline.".into());
    }
    if crate::browser_data::requested(request) {
        let receipt = crate::browser_data::connected(shared, chat, request, body)?;
        let mut result = json!({"isError":false,"structuredContent":receipt});
        super::output::sync_text(&mut result);
        return Ok(json!({"vwMcpResult":result}));
    }
    let directory = folder(shared, chat)?;
    let bytes = serde_json::to_vec(body)
        .map_err(|_| "Not saved: the read could not be encoded. Omit saveTo for an inline read.")?;
    // A saved read is another agent-visible answer. Apply the listener's stored-value scrub
    // before any bytes reach disk, including exports resumed from the frozen receipt cache.
    let bytes = vw_secrets::scrub(&bytes).map_or(bytes, |(clean, _)| clean);
    let mut random = [0u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| "Not saved: a unique filename could not be allocated. Try saving again.")?;
    let name = format!("chrome-read-{:032x}.json", u128::from_be_bytes(random));
    write(&directory, &name, &bytes).map_err(|_| {
        "Not saved: the JSON file could not be written in the chat folder. Choose a writable folder or omit saveTo for an inline read."
    })?;
    let mut receipt = json!({
        "savedToFile":{"path":name,"bytes":bytes.len(),"mime":"application/json"},
        "untrustedPageContent":{"saved":true}
    });
    for key in [
        "operationId",
        "sessionId",
        "profileId",
        "status",
        "action",
        "capability",
        "policyRevision",
        "leaseEpoch",
        "note",
        "promptGuard",
        "sessionReopened",
    ] {
        if let Some(value) = body.get(key) {
            receipt[key] = value.clone();
        }
    }
    // Cursors and coverage remain page data. Saving one read makes no completeness claim about the page.
    for key in ["continuation", "coverage", "readId", "documentId", "tabId"] {
        if let Some(value) = body["untrustedPageContent"]["data"].get(key) {
            receipt["untrustedPageContent"][key] = value.clone();
        }
    }
    let mut result = json!({"isError":false,"structuredContent":receipt});
    super::output::sync_text(&mut result);
    Ok(json!({"vwMcpResult":result}))
}

pub(super) fn write(directory: &Dir, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = directory.open_with(name, &options)?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = directory.remove_file(name);
    }
    result
}
