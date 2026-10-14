//! Last browser-answer filter at the agent tool boundary. Page scripts hide labelled values for useful capture
//! refs; this pass covers every text-bearing result, including action receipts, navigation and page errors.
//!
//! Depends on: `serde_json`. Called by: the agent tool server after the stored-value scrub. A key-page context hides every
//! long ASCII token in the complete answer, even if the page copied it into a different result field.

use serde_json::Value;

const HIDDEN: &str = "[looks like a key: hidden]";

fn key_context(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "api key",
        "api-key",
        "api_key",
        "apikey",
        "access key",
        "client secret",
        "token",
        "secret",
    ]
    .iter()
    .any(|label| lower.contains(label))
        || lower
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| word == "key" || word == "keys")
        || lower.contains("/keys/")
        || lower.ends_with("/keys")
}

fn has_context(value: &Value) -> bool {
    match value {
        Value::String(text) => key_context(text),
        Value::Array(items) => items.iter().any(has_context),
        Value::Object(fields) => {
            fields.get("keyLike").and_then(Value::as_bool) == Some(true)
                || fields
                    .iter()
                    .filter(|(name, _)| name.as_str() != "vwImage")
                    .any(|(_, value)| has_context(value))
        }
        _ => false,
    }
}

fn token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn known(token: &str) -> bool {
    let prefix = [
        "sk-",
        "sk_",
        "pk-",
        "pk_",
        "rk-",
        "rk_",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "xox",
        "AKIA",
        "AIza",
        "glpat-",
        "hf_",
    ];
    prefix.iter().any(|start| token.starts_with(start))
}

fn plausible(token: &str, suffix: &str, context: bool) -> bool {
    if token.len() < 16 {
        return false;
    }
    // On a key page, every long run is withheld, including letters-only, numeric, low-entropy and hex values.
    if context {
        return true;
    }
    if known(token) {
        return true;
    }
    if [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".avif"]
        .iter()
        .any(|ext| suffix.to_ascii_lowercase().starts_with(ext))
    {
        return false;
    }
    if token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    token.len() >= 32
        && token.bytes().any(|b| b.is_ascii_lowercase())
        && token.bytes().any(|b| b.is_ascii_uppercase())
        && token.bytes().any(|b| b.is_ascii_digit())
}

fn candidates(text: &str, context: bool) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !token_char(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && token_char(bytes[index]) {
            index += 1;
        }
        if plausible(&text[start..index], &text[index..], context) {
            found.push(&text[start..index]);
        }
    }
    for (start, _) in text.match_indices("eyJ") {
        let mut end = start;
        while end < bytes.len() && (token_char(bytes[end]) || bytes[end] == b'.') {
            end += 1;
        }
        let parts: Vec<_> = text[start..end].split('.').collect();
        if parts.len() == 3 && parts.iter().all(|part| part.len() >= 8) {
            found.push(&text[start..end]);
        }
    }
    found
}

// These fields come from browser state, capture file creation or fixed host wording, never page strings.
// Design type levels contain our derived role names and parsed numbers; their values/fonts still filter.
// Keep the exception narrow: page-supplied copies under untrustedPageContent still filter.
fn host_metadata(path: &[String]) -> bool {
    match path {
        [field] => matches!(
            field.as_str(),
            "snapshot" | "note" | "note2" | "typingTracker" | "Outcome" | "folder" | "path"
        ),
        [parent, field] if parent == "files" => field == "name",
        [parent, field] if matches!(parent.as_str(), "tab" | "tabs") => field == "id",
        [parent, field] if parent == "change" => field == "summary",
        [outer, parent, field] if outer == "summary" && parent == "design" => field == "typeLevels",
        [outer, parent, field]
            if outer == "untrustedPageContent" && matches!(parent.as_str(), "tab" | "tabs") =>
        {
            field == "id"
        }
        _ => false,
    }
}

fn collect(value: &Value, found: &mut Vec<String>, context: bool, path: &mut Vec<String>) {
    if host_metadata(path) {
        return;
    }
    match value {
        Value::String(text) => {
            found.extend(candidates(text, context).into_iter().map(str::to_owned));
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect(item, found, context, path)),
        Value::Object(fields) => {
            for (name, value) in fields.iter().filter(|(name, _)| name.as_str() != "vwImage") {
                path.push(name.clone());
                collect(value, found, context, path);
                path.pop();
            }
        }
        _ => {}
    }
}

fn replace(value: &mut Value, tokens: &[String], path: &mut Vec<String>) {
    if host_metadata(path) {
        return;
    }
    match value {
        Value::String(text) => {
            for token in tokens {
                *text = text.replace(token, HIDDEN);
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| replace(item, tokens, path)),
        Value::Object(fields) => {
            for (name, value) in fields
                .iter_mut()
                .filter(|(name, _)| name.as_str() != "vwImage")
            {
                path.push(name.clone());
                replace(value, tokens, path);
                path.pop();
            }
        }
        _ => {}
    }
}

/// Filters known and generic keys in every browser result, plus every long token on key pages.
pub fn filter_agent_output(value: &mut Value) {
    let context = has_context(value);
    let mut tokens = Vec::new();
    collect(value, &mut tokens, context, &mut Vec::new());
    tokens.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    tokens.dedup();
    replace(value, &tokens, &mut Vec::new());
}
