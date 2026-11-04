//! Size-measured JSON fragments for the app receipt cache; preserves Unicode and original broker identities.
//! Used only by `output_cache`, with the original page text remaining inside the untrusted-content boundary.
use serde_json::{Value, json};

pub(super) fn header(body: &Value, limit: usize) -> Value {
    let mut header = json!({"note":vw_browser::observe::UNTRUSTED_NOTE,
        "outputLimit":format!("Chrome output capped at {limit} bytes. Frozen receipt: concatenate details.text in offset order (UTF-16 units), then parse the original JSON before using fields or helper continuations. Do not replay writes.")});
    for key in [
        "operationId",
        "sessionId",
        "sessionReopened",
        "profileId",
        "error",
        "status",
        "state",
        "action",
        "capability",
        "policyRevision",
        "leaseEpoch",
        "promptGuard",
    ] {
        if let Some(value) = body.get(key) {
            header[key] = value.clone();
        }
    }
    if let Some(steps) = body["steps"].as_array() {
        header["steps"] = json!(
            steps
                .iter()
                .map(|step| json!({"step":step["step"],"result":{
            "operationId":step["result"]["operationId"],"status":step["result"]["status"],
            "action":step["result"]["action"],"untrustedPageContent":{"buffered":true}}}))
                .collect::<Vec<_>>()
        );
    }
    header
}

pub(super) fn page(
    header: &Value,
    encoded: &str,
    offset: usize,
    continuation: impl Fn(usize) -> Value,
) -> Value {
    let mut end = (offset + 1800).min(encoded.len());
    while !encoded.is_char_boundary(end) {
        end -= 1;
    }
    loop {
        let mut body = header.clone();
        if offset > 0 {
            body.as_object_mut().map(|body| body.remove("steps"));
        }
        body["untrustedPageContent"] = json!({"frozen":true,"details":{
            "path":"", "encoding":"json", "offset":encoded[..offset].encode_utf16().count(),
            "text":&encoded[offset..end],"complete":end == encoded.len(),
            "nextOffset":encoded[..end].encode_utf16().count()}});
        if end < encoded.len() {
            body["continuation"] = continuation(end);
        }
        let limit = if body["steps"].is_array() { 9000 } else { 4000 };
        if super::output::fits(&body, limit) || end == offset {
            return body;
        }
        end = offset + (end - offset) / 2;
        while !encoded.is_char_boundary(end) {
            end -= 1;
        }
    }
}
