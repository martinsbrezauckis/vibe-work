//! `session/history/list` in the fake server: backward pages return the newest `limit` entries
//! before `page.cursor` (an entry id), oldest first within the page; `nextCursor` is the id of the page's first (oldest)
//! entry while older ones exist, else `null`. An unknown session answers `not_found`.
//! `--history-log <file>` appends one line per request (`<sessionId> <cursor|->`), so a test can see that a long
//! export read every page.

use std::io::Write as _;
use std::sync::OnceLock;

static LOG: OnceLock<Option<String>> = OnceLock::new();
static HOLDERS: OnceLock<Option<String>> = OnceLock::new();

/// Sets the lease-holder file (from `--lease-holders`; once).
pub fn set_holders(path: Option<String>) {
    let _ = HOLDERS.set(path);
}

/// The `leaseHolder` of `session_id`: `{processId}` when the holder file names it, else `null`.
pub fn lease_holder(session_id: &str) -> Value {
    let Some(Some(path)) = HOLDERS.get() else {
        return Value::Null;
    };
    let map: Value = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    map.get(session_id)
        .and_then(Value::as_u64)
        .map_or(Value::Null, |pid| json!({"processId": pid}))
}

/// Sets the request log (from the command line; once).
pub fn set_log(path: Option<String>) {
    let _ = LOG.set(path);
}

use serde_json::{Value, json};

/// Answers one `session/history/list` for `state` (`None`: unknown session).
pub fn list(state: Option<&Value>, params: &Value) -> Value {
    let cursor = params.pointer("/page/cursor").and_then(Value::as_str);
    if let Some(Some(path)) = LOG.get()
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = writeln!(
            file,
            "{} {}",
            params["sessionId"].as_str().unwrap_or_default(),
            cursor.unwrap_or("-")
        );
    }
    let Some(state) = state else {
        return json!({"__error": "not_found"});
    };
    let all: Vec<Value> = state["history"].as_array().cloned().unwrap_or_default();
    let limit = usize::try_from(
        params
            .pointer("/page/limit")
            .and_then(Value::as_u64)
            .unwrap_or(200),
    )
    .unwrap_or(200)
    .max(1);
    let end = cursor
        .and_then(|c| all.iter().position(|e| e["id"].as_str() == Some(c)))
        .unwrap_or(all.len());
    let start = end.saturating_sub(limit);
    let items = &all[start..end];
    let next = (start > 0)
        .then(|| items.first().and_then(|e| e["id"].as_str()))
        .flatten();
    json!({"items": items, "nextCursor": next, "previousCursor": null})
}
