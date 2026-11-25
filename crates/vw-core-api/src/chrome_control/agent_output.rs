//! Adapts connected-browser MCP answers to the common browser filter at the agent listener's exit.
//! Depends on vw-browser and the output budgets; preserves opaque bindings and binary content, never page text.
use serde_json::{Value, json};

const MALFORMED: &str = "Result withheld: the browser returned no structured result. Narrow the read; do not replay writes.";
const RECEIPT: &[&str] = &[
    "operationId",
    "requestId",
    "clientId",
    "clientSessionId",
    "browserSessionId",
    "sessionId",
    "profileId",
    "status",
    "state",
    "effect",
    "action",
    "capability",
    "policyRevision",
    "leaseEpoch",
];
const BINDINGS: &[&str] = &[
    "sessionId",
    "profileId",
    "browserSessionId",
    "documentId",
    "childDocumentId",
    "readId",
    "ref",
    "frameId",
    "parentFrameId",
    "tabId",
    "windowId",
    "groupId",
    "pointId",
    "recordingId",
    "selectionId",
    "fileId",
    "mediaId",
    "cursor",
];
// Only these producer paths have bare protocol addresses. Other containers retain named bindings.
const CONTAINERS: &[(&str, &[&str])] = &[
    ("refs", &[]),
    ("frames", &[]),
    ("tabs", &[]),
    ("matches", &[]),
    ("partialMatches", &[]),
    ("elements", &[]),
    ("children", &[]),
    ("target", &[]),
    ("captureIdentity", &[]),
    ("page", &[]),
    ("profiles", &[]),
    ("sessions", &[]),
    ("details", &[]),
    ("structure", &[]),
    ("changes", &[]),
    ("appeared", &[]),
    ("changed", &[]),
    ("disappeared", &[]),
    ("media", &[]),
    ("files", &[]),
    ("currentFiles", &[]),
    ("downloads", &["id"]),
    ("requests", &["id"]),
    ("bookmarks", &["id", "parentId"]),
    ("items", &[]),
    ("nodes", &[]),
    ("binding", &[]),
    ("bindings", &[]),
    ("receipt", &[]),
    ("receipts", &[]),
    ("tab", &[]),
    ("frame", &[]),
    ("element", &[]),
    ("browserTab", &[]),
    ("selection", &[]),
    ("windows", &[]),
    ("window", &[]),
    ("groups", &[]),
    ("group", &[]),
    ("points", &[]),
    ("point", &[]),
    ("dialog", &["id"]),
    ("dialogs", &[]),
    ("selectedProfile", &[]),
    ("profileChoices", &[]),
];
const PROTOCOL_FIELDS: &[&str] = &[
    "scope",
    "matcher",
    "matchKind",
    "resetReason",
    "verification",
    "permissionState",
    "selectionState",
    "stateCode",
    "desiredState",
    "suspensionReason",
];
type Held = Vec<(String, Value)>;

const HIDDEN: &str = "[looks like a key: hidden]";
// These exact ordinary MIME literals contain long runs. No prefix, suffix, parameters or case folding
// are allowed: a variable vendor subtype must still pass the shared key detector.
const FIXED_MIMES: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
];

fn mime_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn mime(value: &mut Value, prefix: &str, held: &mut Held) {
    let Some(value) = value.get_mut("mimeType").filter(|value| value.is_string()) else {
        return;
    };
    let text = value.as_str().unwrap_or_default();
    // RFC 9110 tokens, with parameters deliberately excluded. Syntax alone cannot bless a secret.
    let syntax = text
        .split_once('/')
        .is_some_and(|(kind, subtype)| mime_token(kind) && mime_token(subtype));
    let mut probe = json!({"keyLike":true,"value":text});
    vw_browser::filter_agent_output(&mut probe);
    if syntax && probe["value"] == *value {
        return;
    }
    let mut replacement = json!(HIDDEN);
    if syntax && FIXED_MIMES.contains(&text) {
        // Keep normalization and stored-value scrubbing even for fixed MIME literals.
        let mut ordinary = json!({"value":text});
        vw_browser::filter_agent_output(&mut ordinary);
        if ordinary["value"] == *value {
            replacement = value.clone();
        }
    }
    // Leave the original in the shared filter's context: a MIME containing "secret" must still
    // trigger filtering of sibling page strings. Restore only the validated literal or marker.
    held.push((format!("{prefix}/mimeType"), replacement));
}

fn hold(value: &mut Value, pointer: String, held: &mut Held) {
    held.push((pointer, value.take()));
}

fn fields(value: &mut Value, prefix: &str, names: &[&str], held: &mut Held) {
    for name in names {
        if let Some(value) = value.get_mut(*name).filter(|value| value.is_string()) {
            hold(value, format!("{prefix}/{name}"), held);
        }
    }
}

fn identifier_lists(value: &mut Value, prefix: &str, names: &[&str], held: &mut Held) {
    for name in names {
        if let Some(value) = value.get_mut(*name).filter(|value| {
            value
                .as_array()
                .is_some_and(|rows| rows.iter().all(|row| row.is_string() || row.is_number()))
        }) {
            hold(value, format!("{prefix}/{name}"), held);
        }
    }
}

// Follow protocol containers only. Arbitrary script results, values, console arguments and page copies
// do not acquire an exemption by using a binding's field name. The catalog defines action inputs;
// observations, saved reads and recording receipts supply the corresponding answer containers.
fn bindings(value: &mut Value, prefix: &str, ids: &[&str], held: &mut Held) {
    if let Some(rows) = value.as_array_mut() {
        for (index, row) in rows.iter_mut().enumerate() {
            bindings(row, &format!("{prefix}/{index}"), ids, held);
        }
        return;
    }
    fields(value, prefix, RECEIPT, held);
    fields(value, prefix, BINDINGS, held);
    fields(value, prefix, PROTOCOL_FIELDS, held);
    mime(value, prefix, held);
    if let Some(coverage) = value.get_mut("coverage") {
        let path = format!("{prefix}/coverage");
        fields(coverage, &path, &["reason"], held);
        identifier_lists(coverage, &path, &["sourceLimits"], held);
    }
    identifier_lists(
        value,
        prefix,
        &["removedRefs", "disappeared", "fileIds"],
        held,
    );
    fields(value, prefix, ids, held);
    for name in ["error", "hostError", "warning"] {
        if let Some(error) = value.get_mut(name) {
            fields(error, &format!("{prefix}/{name}"), &["code"], held);
        }
    }
    for &(name, named_ids) in CONTAINERS {
        let Some(child) = value.get_mut(name) else {
            continue;
        };
        // Only network.list emits bare row IDs under data.items. Never inherit an address
        // exemption into arbitrary descendants (DOM id attributes and page objects are content).
        let child_ids = if name == "items" && prefix.ends_with("/data") && ids == ["id"] {
            ids
        } else {
            named_ids
        };
        bindings(child, &format!("{prefix}/{name}"), child_ids, held);
        // Field/encoding select how to reassemble a captured detail; text is page content.
        if name == "details"
            && let Some(rows) = child.as_array_mut()
        {
            for (index, row) in rows.iter_mut().enumerate() {
                fields(
                    row,
                    &format!("{prefix}/details/{index}"),
                    &["field", "encoding"],
                    held,
                );
            }
        }
    }

    if let Some(value) = value.get_mut("continuation") {
        continuation(value, &format!("{prefix}/continuation"), held);
    }
}

fn continuation(value: &mut Value, prefix: &str, held: &mut Held) {
    fields(value, prefix, &["cursor", "readId", "action"], held);
    let Some(map) = value.as_object_mut() else {
        return;
    };
    for (name, child) in map {
        let escaped = name.replace('~', "~0").replace('/', "~1");
        let path = format!("{prefix}/{escaped}");
        if name == "args" {
            // Current producers return generated cursor/tab bindings and numeric budgets. Extra
            // strings (for example a future page-derived query) remain content, even on new streams.
            fields(child, &path, &["cursor"], held);
        } else if child.is_object() {
            continuation(child, &path, held);
        }
    }
}

// Discovery schemas and action-name tables are protocol metadata, not browser page observations.
fn schemas(value: &mut Value, prefix: &str, held: &mut Held) {
    match value {
        Value::Object(map) => {
            for (name, value) in map {
                let escaped = name.replace('~', "~0").replace('/', "~1");
                let pointer = format!("{prefix}/{escaped}");
                if matches!(
                    name.as_str(),
                    "inputSchema"
                        | "inputSchemaJson"
                        | "featureLists"
                        | "features"
                        | "actionRows"
                        | "capabilityNames"
                        | "modes"
                ) {
                    hold(value, pointer, held);
                } else {
                    if matches!(name.as_str(), "action" | "capability") && value.is_string() {
                        hold(value, pointer.clone(), held);
                    }
                    schemas(value, &pointer, held);
                }
            }
        }
        Value::Array(rows) => {
            for (index, row) in rows.iter_mut().enumerate() {
                schemas(row, &format!("{prefix}/{index}"), held);
            }
        }
        _ => {}
    }
}

fn shield(body: &mut Value, prefix: &str, tool: &str, held: &mut Held) {
    if body["untrustedPageContent"]["frozen"] == true {
        // output_cache::bound checked the complete value before serialization. Keep fragments exact,
        // but leave this copy in the filter's context so sibling receipts and resource text still filter.
        held.push((prefix.to_owned(), body.clone()));
        return;
    }
    // Only the exact app-generated action correction is metadata. Helper/page error messages still filter.
    if body.get("untrustedPageContent").is_none()
        && body["error"]["code"] == "UNKNOWN_ACTION"
        && let Some(canonical) = super::output_status::unknown_action("")
        && body["error"] == canonical["structuredContent"]["error"]
        && let Some(error) = body.get_mut("error")
    {
        hold(error, format!("{prefix}/error"), held);
    }
    let action = body["action"].as_str().unwrap_or_default().to_owned();
    bindings(body, prefix, &[], held);
    if let Some(guard) = body.get_mut("promptGuard") {
        // guard_json contains only app-owned signal IDs and channels, never matched page text.
        hold(guard, format!("{prefix}/promptGuard"), held);
    }
    if body["note"] == vw_browser::observe::UNTRUSTED_NOTE {
        fields(body, prefix, &["note"], held);
    }
    for name in ["error", "savedToFile", "receiptDefaults"] {
        if let Some(value) = body.get_mut(name) {
            let names = match name {
                "error" => &["code"][..],
                "savedToFile" => &["path"][..],
                _ => RECEIPT,
            };
            fields(value, &format!("{prefix}/{name}"), names, held);
        }
    }
    if let Some(payload) = body.get_mut("untrustedPageContent") {
        let prefix = format!("{prefix}/untrustedPageContent");
        bindings(payload, &prefix, &[], held);
        fields(
            payload,
            &prefix,
            &["ownerPublicKey", "ownerFingerprint"],
            held,
        );
        if let Some(owner) = payload.get_mut("owner") {
            fields(
                owner,
                &format!("{prefix}/owner"),
                &["key", "publicKey", "fingerprint", "clientId"],
                held,
            );
        }
        if !action.is_empty()
            && action != "raw.execute"
            && !action.starts_with("webmcp.")
            && let Some(data) = payload.get_mut("data")
        {
            let ids: &[&str] = if action.starts_with("bookmarks.") {
                &["id", "parentId"]
            } else if action.starts_with("network.") {
                &["id"]
            } else {
                &[]
            };
            bindings(data, &format!("{prefix}/data"), ids, held);
        }
        if tool == "chrome_status" {
            schemas(payload, &prefix, held);
        }
    }
    if let Some(steps) = body.get_mut("steps").and_then(Value::as_array_mut) {
        for (index, step) in steps.iter_mut().enumerate() {
            if let Some(result) = step.get_mut("result") {
                shield(
                    result,
                    &format!("{prefix}/steps/{index}/result"),
                    tool,
                    held,
                );
            }
        }
    }
}

fn restore(value: &mut Value, held: Held) -> Result<(), &'static str> {
    for (pointer, original) in held {
        *value.pointer_mut(&pointer).ok_or(MALFORMED)? = original;
    }
    Ok(())
}

/// Filters a complete receipt using the same protocol exclusions as the inline agent boundary.
pub(super) fn filter_complete(value: &mut Value, tool: &str) -> Result<(), &'static str> {
    let mut held = Vec::new();
    shield(value, "", tool, &mut held);
    vw_browser::filter_agent_output(value);
    restore(value, held)
}

/// Applies the built-in browser's exact filter after the stored-value scrub, then rebuilds the MCP text copy.
pub(crate) fn filter(value: &mut Value, tool: &str) -> Result<(), &'static str> {
    // Some core-owned outcomes use the ordinary tool-answer shape.
    if value.get("vwMcpResult").is_none() {
        return filter_complete(value, tool);
    }
    let result = value
        .get_mut("vwMcpResult")
        .filter(|value| value.is_object())
        .ok_or(MALFORMED)?;
    if result.get("structuredContent").is_none()
        && result["_meta"]["vwAskOutcome"]["type"] == "refused"
        && result["isError"] == false
    {
        // Confirmed No answers are text-only MCP results from approval_notice, with no browser payload.
        vw_browser::filter_agent_output(result);
        return Ok(());
    }
    let body = result.get_mut("structuredContent").ok_or(MALFORMED)?;
    let mut body = body.take();
    let mut held = Vec::new();
    shield(&mut body, "", tool, &mut held);
    let mut blocks: Vec<Value> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] != "text")
        .cloned()
        .collect();
    let mut binary = Vec::new();
    for (index, block) in blocks.iter_mut().enumerate() {
        let kind = block["type"].as_str().unwrap_or_default().to_owned();
        fields(block, &format!("/{index}"), &["type"], &mut binary);
        // No current producer mints resource URIs or block media IDs. Future producers must
        // establish their provenance before either field can acquire a protocol exemption.
        mime(block, &format!("/{index}"), &mut binary);
        if kind == "resource"
            && let Some(resource) = block.get_mut("resource")
        {
            mime(resource, &format!("/{index}/resource"), &mut binary);
        }
        let path = match kind.as_str() {
            "image" | "audio" => "/data",
            "resource" => "/resource/blob",
            _ => continue,
        };
        if let Some(data) = block.pointer_mut(path) {
            hold(data, format!("/{index}{path}"), &mut binary);
        }
    }
    // Arrays keep the built-in filter's root metadata paths while sharing context with non-text blocks.
    let mut filtered = json!([body, {"content":blocks}]);
    vw_browser::filter_agent_output(&mut filtered);
    restore(&mut filtered[0], held)?;
    restore(&mut filtered[1]["content"], binary)?;
    result["structuredContent"] = filtered[0].take();
    result["content"] = filtered[1]["content"].take();
    super::output::sync_text(result);
    if tool == "chrome_status" {
        super::output::guard_status(result);
    } else {
        super::output::guard(result);
    }
    Ok(())
}

#[cfg(test)]
#[path = "agent_output_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_output_content_tests.rs"]
mod content_tests;
