//! Final Chrome text budgets, including page markers and JSON indentation; used by gateway, batches and vault receipts.
//! Only payload text/list tails shrink. Receipt outcomes, schemas and ref/document bindings never change.
use serde_json::{Value, json};

#[path = "output_status_budget.rs"]
mod status_budget;

pub(super) const LIMIT: usize = 4000;
/// One useful page read, including its envelope, stays below the agent's 16 KB code-mode return.
pub(super) const READ_LIMIT: usize = 12_000;
/// The app's own page.read defaults, sent on every inline read that omits them (cursor reads and exports excepted).
/// The agent-facing action schema states these values; explicit file exports keep the helper's own defaults.
pub(super) const READ_DEFAULT_LIMIT: u64 = 32;
pub(super) const READ_DEFAULT_MAX_BYTES: u64 = 10_000;
pub(super) const READ_DEFAULT_MAX_CHARS: u64 = 10_000;
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
    let pretty_limit = if limit <= LIMIT { 6000 } else { 15_000 };
    if body.to_string().len() > limit {
        return false;
    }
    // A larger read gets one code-mode return. Count its wrapper's indentation, not only the inner JSON.
    let rendered = if limit == READ_LIMIT {
        serde_json::to_string_pretty(&json!({"result":{"reads":[body]}}))
    } else {
        serde_json::to_string_pretty(body)
    };
    rendered.is_ok_and(|s| s.len() <= pretty_limit)
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
        fields
            .entry("maxBytes")
            .or_insert(json!(READ_DEFAULT_MAX_BYTES));
        fields.entry("limit").or_insert(json!(READ_DEFAULT_LIMIT));
        fields
            .entry("maxChars")
            .or_insert(json!(READ_DEFAULT_MAX_CHARS));
    }
    let mut clamped = false;
    for key in ["maxChars", "maxBytes"] {
        if args["args"][key]
            .as_u64()
            .is_some_and(|n| n > READ_LIMIT as u64)
        {
            args["args"][key] = json!(READ_LIMIT);
            clamped = true;
        }
    }
    clamped
}

pub(super) fn clamped(result: &mut Value) {
    if result["structuredContent"].is_object() {
        remove_helper_hint(&mut result["structuredContent"]);
        result["structuredContent"]["outputLimit"] = json!(limit_note(READ_LIMIT));
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

pub(super) fn limit(body: &Value) -> usize {
    if body["steps"].is_array() {
        GROUP_LIMIT
    } else if body["action"] == "page.read" || body["untrustedPageContent"]["action"] == "page.read"
    {
        READ_LIMIT
    } else {
        LIMIT
    }
}

pub(super) fn guard(result: &mut Value) {
    let limit = limit(&result["structuredContent"]);
    guard_with_limit(result, limit, &limit_note(limit));
}

/// The status's index tables: other fields hold positions into them (`{"list":n}`, capability, mode and row indexes),
/// so a shortened table would make those positions point at nothing or at the wrong entry.
const STATUS_TABLES: [&str; 7] = [
    "/untrustedPageContent/profiles",
    "/untrustedPageContent/profileDefaults",
    "/addons",
    "/untrustedPageContent/featureLists",
    "/untrustedPageContent/capabilities/capabilityNames",
    "/untrustedPageContent/capabilities/actionRows",
    "/untrustedPageContent/capabilities/modes",
];

/// Keeps connected profiles, versions and index tables whole before trimming lower-priority status text.
/// Profiles inherit version facts from profileDefaults, so both must survive together with their feature indexes.
pub(super) fn guard_status(result: &mut Value) {
    let note = format!(
        "Browser status capped at {GROUP_LIMIT} bytes. Use chrome_status({{action, profileId}}) for the supported action schema."
    );
    // Own refusals keep their plain, trusted explanation; add-on diagnostics do not turn it into page JSON.
    if result["structuredContent"]
        .get("untrustedPageContent")
        .is_none()
    {
        guard_with_limit(result, GROUP_LIMIT, &note);
        return;
    }
    // The trim below runs against the budget left after the tables. When the tables are omitted afterwards, the
    // status is trimmed again from this untrimmed copy, so session rows and other facts are not cut for tables
    // that the agent never receives.
    let untrimmed = result.clone();
    let mut tables = Vec::new();
    if let Some(body) = result.get_mut("structuredContent") {
        for path in STATUS_TABLES {
            if let Some(table) = body.pointer_mut(path) {
                tables.push((path, table.take()));
            }
        }
    }
    let reserved: usize = tables
        .iter()
        .map(|(_, table)| table.to_string().len())
        .sum();
    guard_with_limit(result, GROUP_LIMIT.saturating_sub(reserved), &note);
    if tables.is_empty() {
        return;
    }
    if let Some(body) = result.get_mut("structuredContent") {
        for (path, table) in tables {
            if let Some(slot) = body.pointer_mut(path) {
                *slot = table;
            }
        }
    }
    if !fits(&result["structuredContent"], GROUP_LIMIT) {
        let mut retry = untrimmed;
        if status_budget::omit_capability_tables(&mut retry["structuredContent"]) {
            // The omission is itself the cap, also when nothing else needs trimming afterwards.
            retry["structuredContent"]["outputLimit"] = json!(note);
            *result = retry;
            guard_status(result);
            return;
        }
    }
    sync_text(result);
}

fn guard_with_limit(result: &mut Value, limit: usize, note: &str) {
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
    super::output_batch::share_context(body, &[note, &limit_note(LIMIT), &limit_note(READ_LIMIT)]);
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
            // A status's session rows are the chat's own leases (at most one per browser): only their text shrinks.
            let min = usize::from(path.ends_with("/refs"));
            if rows.len() > min
                && !path.ends_with("/steps")
                && !path.ends_with("/frames")
                && path != "/untrustedPageContent/sessions"
            {
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
