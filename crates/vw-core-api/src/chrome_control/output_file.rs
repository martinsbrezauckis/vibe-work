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

/// Upper bound on the receipts one export follows; the collection's own 10000-row limit still applies.
const EXPORT_PAGES: usize = 400;
const EXPORT_SECTIONS: usize = 10_001;

/// A table or list export through a connected browser arrives in bounded receipts: the helper keeps each receipt
/// small and Safari pages its captured structure. Before saving, follow the read's own continuations (the helper's
/// receipt cursor first, then the captured structure cursor) and gather every structure row into the first receipt.
/// Coverage is complete only when no continuation remains and no receipt reported a partial capture; a failed or
/// cancelled continuation keeps the remaining cursor, so the saved file says it is incomplete.
pub(super) async fn send(
    shared: &std::sync::Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
    consent: Option<&super::read_consent::Consent>,
) -> Result<super::owner::Answer, String> {
    let request = args.clone();
    let mut args = args;
    // Export options are the app's; the browser only performs the read.
    if let Some(fields) = args.as_object_mut() {
        for key in ["saveTo", "extract", "appendFrom", "deduplicate"] {
            fields.remove(key);
        }
    }
    let mut answer = super::read_consent::send(shared, session, tool, args, consent).await?;
    if tool == "chrome_execute" {
        collect_pages(shared, session, &request, &mut answer, consent).await;
    }
    Ok(answer)
}

async fn collect_pages(
    shared: &std::sync::Arc<Shared>,
    session: &str,
    request: &Value,
    answer: &mut super::owner::Answer,
    consent: Option<&super::read_consent::Consent>,
) {
    if !crate::browser_data::requested(request) {
        return;
    }
    let first = &answer.value()["structuredContent"];
    if first["status"] != "completed" || !first["error"].is_null() {
        return;
    }
    let data = &first["data"];
    let mut partial = partial_capture(data);
    let mut next = structure_continuation(data);
    if next.is_none() {
        return;
    }
    let mut sections: Vec<Value> = data["structure"].as_array().cloned().unwrap_or_default();
    let mut pages = 0;
    while let Some(args) = next.clone() {
        if pages >= EXPORT_PAGES || sections.len() >= EXPORT_SECTIONS {
            break;
        }
        pages += 1;
        let call = json!({"sessionId":request["sessionId"],"action":"page.read","args":args});
        let Ok(page) =
            super::read_consent::send(shared, session, "chrome_execute", call, consent).await
        else {
            break;
        };
        let body = &page.value()["structuredContent"];
        if body["status"] != "completed" || !body["error"].is_null() {
            break;
        }
        let data = &body["data"];
        partial |= partial_capture(data);
        sections.extend(data["structure"].as_array().cloned().unwrap_or_default());
        next = structure_continuation(data);
    }
    let data = &mut answer.value_mut()["structuredContent"]["data"];
    data["structure"] = json!(sections);
    let complete = next.is_none() && !partial;
    data["coverage"]["complete"] = json!(complete);
    if let Some(args) = next {
        data["continuation"] = json!({"structure":{"action":"page.read","args":args}});
    } else if let Some(fields) = data.as_object_mut() {
        fields.remove("continuation");
        if complete
            && let Some(coverage) = fields.get_mut("coverage").and_then(Value::as_object_mut)
        {
            coverage.remove("reason");
        }
    }
}

/// The next receipt of the same capture that can still hold structure rows.
fn structure_continuation(data: &Value) -> Option<Value> {
    ["helper", "structure"].iter().find_map(|key| {
        let args = &data["continuation"][*key]["args"];
        args["cursor"].is_string().then(|| args.clone())
    })
}

/// The capture itself was limited (not merely paged), so even every receipt cannot hold the whole table.
fn partial_capture(data: &Value) -> bool {
    data["scanTruncated"] == true
        || data["coverage"]["sourceLimits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty())
        || data["coverage"]["reason"]
            .as_str()
            .is_some_and(|reason| reason != "captured_content_remaining")
}
