//! Validate proposed log edits with serde before the scanner's atomic replacement.
//! Called by the worker; preserves object keys, types and Vibe's format fields. JSONL is checked per line.

use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
use zeroize::{Zeroize, Zeroizing};

pub(crate) fn safe(original: &Path, candidate: &Path) -> bool {
    let Ok(source) = File::open(original) else {
        return false;
    };
    let Ok(target) = File::open(candidate) else {
        return false;
    };
    if original.extension().is_some_and(|ext| ext == "jsonl") {
        lines(BufReader::new(source), BufReader::new(target))
    } else {
        pair(
            serde_json::from_reader(BufReader::new(source)),
            serde_json::from_reader(BufReader::new(target)),
        )
    }
}

fn lines(mut source: impl BufRead, mut target: impl BufRead) -> bool {
    let mut before = Zeroizing::new(String::new());
    let mut after = Zeroizing::new(String::new());
    loop {
        before.zeroize();
        after.zeroize();
        let (Ok(a), Ok(b)) = (source.read_line(&mut before), target.read_line(&mut after)) else {
            return false;
        };
        if a == 0 || b == 0 {
            return a == b;
        }
        if !pair(serde_json::from_str(&before), serde_json::from_str(&after)) {
            return false;
        }
    }
}

fn pair(source: serde_json::Result<Value>, target: serde_json::Result<Value>) -> bool {
    let (Ok(source), Ok(target)) = (source.map(Parsed), target.map(Parsed)) else {
        return false;
    };
    same(&source.0, &target.0, "")
}
struct Parsed(Value);
impl Drop for Parsed {
    fn drop(&mut self) {
        wipe(self.0.take());
    }
}

fn same(source: &Value, target: &Value, field: &str) -> bool {
    match (source, target) {
        (Value::Object(a), Value::Object(b)) => {
            a.keys().eq(b.keys()) && a.iter().all(|(key, value)| same(value, &b[key], key))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b, field))
        }
        (Value::String(a), Value::String(b)) => !protected(field) || a == b,
        _ => source == target,
    }
}

fn protected(field: &str) -> bool {
    matches!(
        field,
        "role"
            | "type"
            | "id"
            | "session_id"
            | "parent_session_id"
            | "name"
            | "alias"
            | "cwd"
            | "working_directory"
            | "save_dir"
            | "session_prefix"
            | "active_model"
            | "default_model"
            | "default_agent"
            | "agent_name"
            | "model_alias"
            | "provider"
            | "api_base"
            | "start_time"
            | "end_time"
            | "bumped_at"
            | "pinned_at"
            | "updated_at"
            | "created_at"
    )
}

fn wipe(value: Value) {
    match value {
        Value::String(mut text) => text.zeroize(),
        Value::Array(values) => values.into_iter().for_each(wipe),
        Value::Object(values) => {
            for (mut key, value) in values {
                key.zeroize();
                wipe(value);
            }
        }
        _ => {}
    }
}
