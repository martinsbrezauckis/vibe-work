//! Rules applied to forwarded calls that the gate rewrites or tightens: `turn/start` and
//! `turn/steer` (our own `clientUserMessageId`), `session/delete` (native confirmation),
//! `session/turn/enqueue` and `session/turn/queue/replace` (our own `entryId` on every user entry
//! keeps a replaced queue item anchored), user content
//! (`content.rs`: an allowlist of block types, inline bytes only), `session/shellCommand` (running a command needs a native confirmation), and
//! `session/start|resume|fork|continue` (`agentConfig` filter, headless default).
//!
//! Depends on: `agent_config`, ids, picked, canonical, shape. Called by: gate (`plan`).
//! Invariants:
//! - Every user message leaves with a non-empty anchor id: `clientUserMessageId` on `turn/start`
//!   and `turn/steer`, `entryId` on each `role: "user"` entry of `session/turn/enqueue` and
//!   `session/turn/queue/replace`. The
//!   caller's id is kept when it is usable, otherwise one we generate (rewind and fork
//!   resolve their anchor by that id, and the ids Vibe mints itself are not resolvable on the
//!   unified harness).
//! - Everything else in the params is forwarded exactly as given.

use serde_json::{Map, Value, json};

use crate::agent_config::filter;
use crate::canonical::is_clean_text;
use crate::content;
use crate::error::GateError;
use crate::ids::random_hex;
use crate::picked::PickedFolders;
use crate::shape::invalid;

/// Params to send plus what the gate adds to the reply.
pub(crate) struct Adjusted {
    /// The params to send.
    pub(crate) params: Value,
    /// The message id of a `turn/start` or `turn/steer`.
    pub(crate) client_user_message_id: Option<String>,
    /// The `entryId` of each user entry of a `session/turn/enqueue`, in order.
    pub(crate) user_entry_ids: Vec<String>,
    /// Summary when the call needs a native confirmation.
    pub(crate) confirm: Option<String>,
}

/// Methods that create or reopen a session and take an `agentConfig` (`SessionStartParams`,
/// `SessionResumeParams`, `SessionForkParams`, `SessionContinueParams`).
const SESSION_ENTRY_METHODS: &[&str] = &[
    "session/start",
    "session/resume",
    "session/fork",
    "session/continue",
];

/// Applies the rule for `method`, if it has one; other methods pass through untouched.
///
/// Defense in depth: any other forwarded call that carries a top-level `agentConfig` is filtered
/// too (no `headless` default). Typed validation already refuses the key on methods whose
/// generated params lack it, so this only matters if a Vibe update adds the field to one.
pub(crate) fn adjust(
    method: &str,
    params: &Value,
    picked: &PickedFolders,
) -> Result<Adjusted, GateError> {
    content::check(method, params)?;
    if matches!(method, "turn/start" | "turn/steer") {
        return with_message_id(method, params);
    }
    if matches!(
        method,
        "session/turn/enqueue" | "session/turn/queue/replace"
    ) {
        return enqueue(method, params);
    }
    if method == "session/shellCommand" {
        return shell_command(params);
    }
    if method == "session/delete" {
        return Ok(delete_session(params));
    }
    let entry = SESSION_ENTRY_METHODS.contains(&method);
    if entry || params.get("agentConfig").is_some() {
        return session_entry(method, params, picked, entry);
    }
    Ok(Adjusted {
        params: params.clone(),
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
        confirm: None,
    })
}

/// The caller's anchor id under `key` when usable, else a fresh one. Absent, `null` and `""`
/// count as missing; anything else that is not clean text up to 128 characters is refused.
fn anchor_id(method: &str, object: &Map<String, Value>, key: &str) -> Result<String, GateError> {
    match object.get(key) {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s.is_empty() => {}
        Some(Value::String(s)) if is_clean_text(s, 128) => return Ok(s.clone()),
        Some(_) => {
            return Err(invalid(
                method,
                format!("`{key}` must be clean text up to 128 characters"),
            ));
        }
    }
    Ok(format!("vw-msg-{}", random_hex(12)?))
}

/// `turn/start` and `turn/steer`: make sure `clientUserMessageId` is set.
fn with_message_id(method: &str, params: &Value) -> Result<Adjusted, GateError> {
    let mut out = params.clone();
    let Some(object) = out.as_object_mut() else {
        return Err(invalid(method, "params must be an object"));
    };
    let id = anchor_id(method, object, "clientUserMessageId")?;
    object.insert("clientUserMessageId".to_owned(), json!(id));
    Ok(Adjusted {
        params: out,
        client_user_message_id: Some(id),
        user_entry_ids: Vec::new(),
        confirm: None,
    })
}

/// `session/turn/enqueue` and `session/turn/queue/replace`: make sure every `role: "user"` entry has an
/// `entryId`
/// (`TurnUserInputEntry.entryId`, `protocol.py` via the generated `TurnInputEntry`). Context
/// entries carry no anchor and are left alone. The typed check already ran, so `entries` is an
/// array of objects with a `role`.
fn enqueue(method: &str, params: &Value) -> Result<Adjusted, GateError> {
    let mut out = params.clone();
    let entries = out
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid(method, "`entries` must be an array"))?;
    let mut ids = Vec::new();
    for entry in entries.iter_mut() {
        let Some(object) = entry.as_object_mut() else {
            return Err(invalid(method, "each entry must be an object"));
        };
        if object.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let id = anchor_id(method, object, "entryId")?;
        object.insert("entryId".to_owned(), json!(id));
        ids.push(id);
    }
    Ok(Adjusted {
        params: out,
        client_user_message_id: None,
        user_entry_ids: ids,
        confirm: None,
    })
}

/// `session/shellCommand` runs a host shell command directly (Vibe's `_dispatch_shell_command`:
/// no approval callback, no profile check), so any `action` other than `interrupt` needs a native
/// confirmation, bound to the exact params and so to the exact command. `action` absent means
/// `run` (Vibe's default). Interrupting a running command stays free so the user can always stop
/// one. The typed check already pinned `action` to `run`, `interrupt` or absent.
fn shell_command(params: &Value) -> Result<Adjusted, GateError> {
    let interrupt = params.get("action").and_then(Value::as_str) == Some("interrupt");
    if !interrupt
        && params
            .get("command")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(invalid(
            "session/shellCommand",
            "a command is required to run",
        ));
    }
    Ok(Adjusted {
        params: params.clone(),
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
        confirm: (!interrupt).then(|| {
            "Run this shell command now (Vibe runs it directly, without its own approval step)"
                .to_owned()
        }),
    })
}

/// `session/delete` removes the session and its history from the shared store for good (the Vibe
/// CLI loses it too), so it needs a native confirmation bound to the
/// exact session id. The core's dialog text names the session by title (`vw-core-api`), the gate
/// only knows the id.
fn delete_session(params: &Value) -> Adjusted {
    Adjusted {
        params: params.clone(),
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
        confirm: Some(
            "Delete this session and its history permanently (the Vibe CLI loses it too)"
                .to_owned(),
        ),
    }
}

fn session_entry(
    method: &str,
    params: &Value,
    picked: &PickedFolders,
    default_headless: bool,
) -> Result<Adjusted, GateError> {
    let mut out = params.clone();
    let Some(object) = out.as_object_mut() else {
        return Err(invalid(method, "params must be an object"));
    };
    let filtered = filter(method, object.get("agentConfig"), picked, default_headless)?;
    object.insert("agentConfig".to_owned(), filtered.config);
    Ok(Adjusted {
        params: out,
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
        confirm: filtered.confirm,
    })
}
