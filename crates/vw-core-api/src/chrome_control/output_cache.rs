//! App-owned frozen receipt buffer, outside helper/extension cursors; called by the gateway and batch finisher.
//! Binds every page to its chat, live session and original permission; stores only bounded, expiring memory.
use crate::core::Shared;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::PoisonError,
    time::{Duration, Instant},
};

const PREFIX: &str = "vw-output:";
const MAX_BYTES: usize = 64 * 1024 * 1024;
const TTL: Duration = Duration::from_secs(300);

pub(super) fn is_continuation(request: &Value) -> bool {
    request["args"]["cursor"]
        .as_str()
        .is_some_and(|cursor| cursor.starts_with(PREFIX))
}

struct Entry {
    id: String,
    chat: String,
    request: Value,
    created: Instant,
    encoded: String,
    header: Value,
    is_error: Value,
}

#[derive(Default)]
pub(super) struct Cache {
    entries: VecDeque<Entry>,
}

impl Cache {
    pub(super) fn clear_chat(&mut self, chat: &str) {
        self.entries.retain(|entry| entry.chat != chat);
    }

    fn expire(&mut self) {
        self.entries.retain(|entry| entry.created.elapsed() < TTL);
    }
}

/// New observations need lossless paging. Legacy offset receipts keep their established guard behavior.
pub(super) fn observation(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key("readId")
                || map.get("summary") == Some(&Value::Bool(true))
                || map.contains_key("continuation")
                || map.contains_key("changes")
                || map.contains_key("matcher")
                || map.values().any(observation)
        }
        Value::Array(rows) => rows.iter().any(observation),
        _ => false,
    }
}

pub(super) fn for_model(
    shared: &Shared,
    chat: &str,
    request: &Value,
    answer: super::owner::Answer,
    tool: &str,
    clamped: bool,
) -> Value {
    let mut result = super::content::for_model(answer);
    super::receipt::stamp(shared, tool, request, &mut result);
    if tool == "chrome_execute"
        && super::output_file::requested(request)
        && result["structuredContent"]["status"] == "completed"
        && result["isError"] != true
        && result["structuredContent"]["error"].is_null()
    {
        return save(shared, chat, &result["structuredContent"]);
    }
    if tool != "chrome_execute" || !observation(&result) {
        return super::output::finish(result, tool, clamped);
    }
    super::output_receipt::compact(&mut result);
    if clamped {
        result["structuredContent"]["requestLimit"] = json!(
            "page.read maxChars/maxBytes capped at 4000 by the app. Use the returned continuation for remaining content."
        );
        super::output::sync_text(&mut result);
    }
    bound(shared, chat, request, &mut result);
    json!({"vwMcpResult":result})
}

/// Save the exact marked body before any extra app trimming, including a combined batch's whole body.
pub(super) fn bound(shared: &Shared, chat: &str, request: &Value, result: &mut Value) {
    let body = &result["structuredContent"];
    let limit = if body["steps"].is_array() { 9000 } else { 4000 };
    if super::output::fits(body, limit) {
        return;
    }
    let encoded = body.to_string();
    let mut random = [0u8; 16];
    if encoded.len() > MAX_BYTES || getrandom::fill(&mut random).is_err() {
        // No incomplete field can masquerade as a complete value if the cache cannot retain it.
        let mut header = super::output_fragment::header(body, limit);
        header["untrustedPageContent"] = json!({"omitted":true});
        header["outputLimit"] = json!(
            "Receipt exceeds the app buffer capacity. Content is unavailable; narrow a read. Do not replay writes."
        );
        result["structuredContent"] = header;
        super::output::sync_text(result);
        return;
    }
    let id = format!("{:032x}", u128::from_be_bytes(random));
    let mut request = request.clone();
    if let Some(session) = body["sessionReopened"]["sessionId"].as_str() {
        request["sessionId"] = json!(session);
    }
    let entry = Entry {
        id,
        chat: chat.to_owned(),
        request,
        created: Instant::now(),
        encoded,
        header: super::output_fragment::header(body, limit),
        is_error: result.get("isError").cloned().unwrap_or(json!(false)),
    };
    result["structuredContent"] = page(&entry, 0);
    let mut cache = shared
        .chrome_control
        .output_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    cache.expire();
    while cache.entries.len() >= 64
        || cache.entries.iter().map(|e| e.encoded.len()).sum::<usize>() + entry.encoded.len()
            > MAX_BYTES
    {
        cache.entries.pop_front();
    }
    cache.entries.push_back(entry);
    super::output::sync_text(result);
}

pub(super) fn resume(
    shared: &Shared,
    chat: &str,
    request: &Value,
) -> Option<Result<Value, String>> {
    let cursor = request["args"]["cursor"].as_str()?;
    let suffix = cursor.strip_prefix(PREFIX)?;
    Some((|| {
        let invalid = "Frozen app receipt expired or does not belong to this session. Make a fresh read; do not replay writes.";
        if request["action"] != "page.read"
            || request["args"]
                .as_object()
                .is_none_or(|args| args.keys().any(|key| key != "cursor"))
        {
            return Err("App continuation takes page.read with only its cursor. Use the returned action and args verbatim.".into());
        }
        let (id, offset) = suffix.rsplit_once(':').ok_or(invalid)?;
        let offset: usize = offset.parse().map_err(|_| invalid)?;
        let mut cache = shared
            .chrome_control
            .output_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cache.expire();
        let entry = cache
            .entries
            .iter()
            .find(|entry| {
                entry.id == id
                    && entry.chat == chat
                    && entry.request["sessionId"] == request["sessionId"]
            })
            .ok_or(invalid)?;
        // Reading a cached write result never executes it, but a mode reduction still revokes access.
        if let Some(steps) = entry.request["steps"].as_array() {
            for step in steps {
                super::check::authorize(shared, chat, "chrome_execute", step)?;
            }
        } else {
            super::check::authorize(shared, chat, "chrome_execute", &entry.request)?;
        }
        if offset >= entry.encoded.len() || !entry.encoded.is_char_boundary(offset) {
            return Err(invalid.into());
        }
        if super::output_file::requested(request) {
            if entry.request["action"] != "page.read" {
                return Err("Not saved: this buffer belongs to another action; omit saveTo to read its receipt inline. Only page.read results support export.".into());
            }
            if entry.is_error == true {
                return Err(
                    "Not saved: this read failed; omit saveTo to read its diagnostic inline."
                        .into(),
                );
            }
            let original: Value = serde_json::from_str(&entry.encoded).map_err(|_| invalid)?;
            drop(cache);
            return super::output_file::save(shared, chat, &original);
        }
        let mut result = json!({"isError":entry.is_error,"structuredContent":page(entry, offset)});
        super::output::sync_text(&mut result);
        Ok(json!({"vwMcpResult":result}))
    })())
}

fn save(shared: &Shared, chat: &str, body: &Value) -> Value {
    super::output_file::save(shared, chat, body).unwrap_or_else(
        |reason| json!({"vwMcpResult":super::owner::failure(&reason, "SAVE_FAILED")}),
    )
}

fn page(entry: &Entry, offset: usize) -> Value {
    super::output_fragment::page(&entry.header, &entry.encoded, offset, |next| {
        json!({"action":"page.read","args":{"cursor":format!("{PREFIX}{}:{next}", entry.id)},
            "reason":"App receipt buffer; finish it before the preserved helper continuation."})
    })
}

#[cfg(test)]
#[path = "output_cache_tests.rs"]
mod tests;
