//! Final Chrome text budgets, including page markers and JSON indentation; used by gateway, batches and vault receipts.
//! Only payload text/list tails shrink. Receipt outcomes, schemas and ref/document bindings never change.
use serde_json::{Value, json};

pub(super) const LIMIT: usize = 4000;
const GROUP_LIMIT: usize = 9000;
const CONTINUATION: &str = "Continue page.read with textOffset=nextTextOffset for text or offset=nextOffset and limit for refs; keep filters. If offsets are absent or do not advance, narrow with find/ref/frameId. Narrow other reads with their schema; do not replay writes.";

fn limit_note(limit: usize) -> String {
    format!("Browser output capped at {limit} bytes. {CONTINUATION}")
}

#[cfg(test)]
pub(super) fn for_model(answer: super::owner::Answer, tool: &str, was_clamped: bool) -> Value {
    finish(super::content::for_model(answer), tool, was_clamped)
}

pub(super) fn finish(mut result: Value, tool: &str, was_clamped: bool) -> Value {
    if tool == "chrome_execute" {
        super::output_receipt::compact(&mut result);
    }
    if was_clamped {
        clamped(&mut result);
    }
    if tool != "chrome_status" {
        guard(&mut result);
    }
    json!({"vwMcpResult":result})
}

pub(super) fn fits(body: &Value, limit: usize) -> bool {
    // Two individual receipts plus the code-mode return object's indentation must fit under 16K.
    let pretty_limit = if limit <= LIMIT { 6000 } else { 15_000 };
    body.to_string().len() <= limit
        && serde_json::to_string_pretty(body).is_ok_and(|s| s.len() <= pretty_limit)
}

pub(super) fn clamp_request(tool: &str, args: &mut Value) -> bool {
    if tool != "chrome_execute" || args["action"] != "page.read" {
        return false;
    }
    if super::output_file::requested(args) {
        return false;
    }
    if args["args"].get("cursor").is_none()
        && let Some(fields) = args["args"].as_object_mut()
    {
        // A text-size request must not implicitly enlarge the helper's byte budget.
        fields.entry("maxBytes").or_insert(json!(2000));
        fields.entry("limit").or_insert(json!(8));
    }
    let mut clamped = false;
    for key in ["maxChars", "maxBytes"] {
        if args["args"][key].as_u64().is_some_and(|n| n > LIMIT as u64) {
            args["args"][key] = json!(LIMIT);
            clamped = true;
        }
    }
    clamped
}

pub(super) fn clamped(result: &mut Value) {
    if result["structuredContent"].is_object() {
        remove_helper_hint(&mut result["structuredContent"]);
        result["structuredContent"]["outputLimit"] = json!(limit_note(LIMIT));
        guard(result);
        sync_text(result);
    }
}

pub(super) fn sync_text(result: &mut Value) {
    let rendered = result["structuredContent"].to_string();
    // A single text block is authoritative. Preserve image/resource blocks unchanged.
    let mut blocks = vec![json!({"type":"text","text":rendered})];
    if let Some(content) = result["content"].as_array() {
        blocks.extend(content.iter().filter(|b| b["type"] != "text").cloned());
    }
    result["content"] = json!(blocks);
}

pub(super) fn guard(result: &mut Value) {
    // The batch wrapper has a separate total limit; each action already used the smaller budget.
    let limit = if result["structuredContent"]["steps"].is_array() {
        GROUP_LIMIT
    } else {
        LIMIT
    };
    guard_with_limit(result, limit);
}

pub(super) fn guard_status(result: &mut Value) {
    guard_with_limit(result, GROUP_LIMIT);
}

fn guard_with_limit(result: &mut Value, limit: usize) {
    let text_bytes: usize = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .map(str::len)
        .sum();
    let Some(body) = result.get_mut("structuredContent") else {
        return;
    };
    if fits(body, limit) {
        if text_bytes > limit {
            sync_text(result);
        }
        return;
    }
    remove_helper_hint(body);
    let note = limit_note(limit);
    super::output_batch::share_context(body, [&note, &limit_note(LIMIT)]);
    body["outputLimit"] = json!(note);
    while !fits(body, limit) {
        let Some((path, _)) = largest(body, "") else {
            if omit_refs(body) || omit_data(body) {
                continue;
            }
            break;
        };
        shrink(body, &path);
    }
    sync_text(result);
}

// Raw script results may use identity-shaped keys for arbitrarily large values. If the protected
// context itself cannot fit, omit that observation as a unit, including all refs, not partial identities.
fn omit_data(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            if map
                .get("data")
                .is_some_and(|data| data.to_string().len() > 32)
            {
                map.insert("data".into(), json!({"omitted":true}));
                return true;
            }
            map.values_mut().any(omit_data)
        }
        Value::Array(rows) => rows.iter_mut().any(omit_data),
        _ => false,
    }
}

// A batch can have sixteen irreducible page contexts. Omit refs together with their frame inventory,
// never leave a retained ref pointing into an omitted or another step's document. The offset explicitly
// does not advance: the continuation note requires a narrower read instead of an empty paging loop.
fn omit_refs(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            if map
                .get("refs")
                .and_then(Value::as_array)
                .is_some_and(|rows| !rows.is_empty())
            {
                map.insert("refs".into(), json!([]));
                if map.contains_key("frames") {
                    map.insert("frames".into(), json!([]));
                }
                if let Some(offset) = map.get("offset").cloned() {
                    map.insert("nextOffset".into(), offset);
                }
                return true;
            }
            map.values_mut().any(omit_refs)
        }
        Value::Array(rows) => rows.iter_mut().any(omit_refs),
        _ => false,
    }
}

fn remove_helper_hint(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // The helper's suggestion to raise maxBytes no longer describes the final app budget.
            if map.contains_key("refs") && map.contains_key("text") {
                map.remove("trimmed");
            }
            for value in map.values_mut() {
                remove_helper_hint(value);
            }
        }
        Value::Array(rows) => {
            for row in rows {
                remove_helper_hint(row);
            }
        }
        _ => {}
    }
}

fn protected(key: &str) -> bool {
    matches!(
        key,
        "operationId"
            | "sessionId"
            | "profileId"
            | "profileName"
            | "profileChoice"
            | "selectedProfile"
            | "status"
            | "effect"
            | "state"
            | "action"
            | "capability"
            | "policyRevision"
            | "leaseEpoch"
            | "documentId"
            | "childDocumentId"
            | "ref"
            | "toRef"
            | "frameId"
            | "tabId"
            | "origin"
            | "frameOrigin"
            | "inputSchema"
            | "promptGuard"
            | "note"
            | "outputLimit"
            | "connection"
            | "code"
            | "role"
            | "tag"
            | "type"
            | "continuation"
            | "cursor"
            | "readId"
            | "revision"
    )
}

fn largest(value: &Value, path: &str) -> Option<(String, usize)> {
    match value {
        Value::Object(map) => map
            .iter()
            .filter(|(key, _)| !protected(key))
            .filter_map(|(key, value)| {
                let key = key.replace('~', "~0").replace('/', "~1");
                largest(value, &format!("{path}/{key}"))
            })
            .max_by_key(|(_, size)| *size),
        Value::Array(rows) => {
            let children = rows
                .iter()
                .enumerate()
                .filter_map(|(i, row)| largest(row, &format!("{path}/{i}")))
                .max_by_key(|(_, size)| *size);
            // Never discard batch outcomes or frame bindings. A ref row keeps its identity, even if its name is huge.
            let min = usize::from(path.ends_with("/refs"));
            if rows.len() > min && !path.ends_with("/steps") && !path.ends_with("/frames") {
                Some((path.to_owned(), value.to_string().len()))
            } else {
                children
            }
        }
        Value::String(text) if text.chars().count() > 1 => Some((path.to_owned(), text.len())),
        _ => None,
    }
}

fn shrink(body: &mut Value, path: &str) {
    let Some((parent, key)) = path.rsplit_once('/') else {
        return;
    };
    let Some(parent) = body.pointer_mut(parent) else {
        return;
    };
    let key = key.replace("~1", "/").replace("~0", "~");
    // A shortened address could be mistaken for a different usable URL. Omit the whole address;
    // the exact ref/document binding remains, and outputLimit explains why metadata is missing.
    if ["url", "href"].contains(&key.as_str())
        && let Some(map) = parent.as_object_mut()
    {
        map.remove(&key);
        map.insert(format!("{key}Omitted"), json!(true));
        return;
    }
    let value = if parent.is_array() {
        key.parse::<usize>()
            .ok()
            .and_then(|index| parent.get_mut(index))
    } else {
        parent.get_mut(&key)
    };
    let Some(value) = value else { return };
    match value {
        Value::Array(rows) => {
            let keep = (rows.len() / 2).max(usize::from(key == "refs"));
            rows.truncate(keep);
            if key == "refs" && parent.get("offset").is_some() {
                parent["nextOffset"] = json!(parent["offset"].as_u64().unwrap_or(0) + keep as u64);
            } else if key == "refs" && parent["delta"] == true {
                // Delta refs are a filtered change list, not a page offset. Request a fresh narrow read.
                parent["moreRefs"] = json!(true);
            }
        }
        Value::String(text) => {
            let keep = text.chars().count() / 2;
            *text = text.chars().take(keep).collect();
            let units = text.encode_utf16().count();
            if key == "text" && parent.get("textOffset").is_some() {
                parent["nextTextOffset"] =
                    json!(parent["textOffset"].as_u64().unwrap_or(0) + units as u64);
            }
        }
        _ => {}
    }
}
