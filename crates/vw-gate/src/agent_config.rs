//! The `agentConfig` filter for `session/start`, `session/resume`, `session/fork` and
//! `session/continue`.
//!
//! Depends on: picked, profile, shape helpers, error. Called by: `session_rules`.
//! Those four are the methods whose generated params type has an `agentConfig` field.
//! Why: `agentConfig` is a second door to code execution and to weakened safety, next to the
//! privileged methods. It can carry stdio MCP servers (a command line), hooks, tool definitions, a
//! sandbox override and `autoApprove`. The webview is untrusted, so only the keys the app needs
//! are accepted, each with its own check; every other key is refused.
//! Invariants:
//! - `headless` defaults to `false` (scheduled loops never fire when headless).
//! - `autoApprove: true` and `agent: "auto-approve"` need a native confirmation.
//! - `trustWorkspace: true` only for a `cwd` inside a folder the user picked natively, and with an
//!   existing `worktree` only when the worktree is inside one too (Vibe trusts the worktree).
//! - Every `workspaceRoots` entry inside a folder the user picked natively: Vibe loads a listed
//!   root's project files without a trust check, so a root is a trust grant.
//! - Valid caller keys are forwarded untouched; the only addition is the `headless` default.

use serde_json::{Map, Value, json};

use crate::canonical::is_clean_text;
use crate::error::{DenyReason, GateError};
use crate::picked::PickedFolders;
use crate::profile::{escalates, is_profile};
use crate::shape::invalid;

/// Result of filtering one `agentConfig`.
pub(crate) struct Filtered {
    /// The `agentConfig` object to send (the caller's keys plus the `headless` default).
    pub(crate) config: Value,
    /// Summary when the config asks for an escalation that needs a native confirmation.
    pub(crate) confirm: Option<String>,
}

fn policy(method: &str, rule: String) -> GateError {
    GateError::Denied {
        method: method.to_owned(),
        reason: DenyReason::Policy(rule),
    }
}

fn is_text_list(value: &Value, max_items: usize, max_len: usize) -> bool {
    value.as_array().is_some_and(|items| {
        items.len() <= max_items
            && items
                .iter()
                .all(|v| v.as_str().is_some_and(|s| is_clean_text(s, max_len)))
    })
}

/// Filters the `agentConfig` of `method`'s params. `raw` is `None` when the caller sent none.
/// `default_headless` adds `headless: false` when the caller did not choose.
pub(crate) fn filter(
    method: &str,
    raw: Option<&Value>,
    picked: &PickedFolders,
    default_headless: bool,
) -> Result<Filtered, GateError> {
    let mut object: Map<String, Value> = match raw {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return Err(invalid(method, "`agentConfig` must be an object")),
    };
    let mut confirm = None;
    for (key, value) in &object {
        let fine = match key.as_str() {
            "cwd" => value.as_str().is_some_and(|s| is_clean_text(s, 4096)),
            "workspaceRoots" => is_text_list(value, 32, 4096),
            "enabledTools" => value.is_null() || is_text_list(value, 256, 128),
            "disabledTools" => is_text_list(value, 256, 128),
            "agent" => match value.as_str() {
                Some(name) if is_profile(name) => {
                    if escalates(name) {
                        confirm = Some(escalation_text());
                    }
                    true
                }
                _ => false,
            },
            "autoApprove" => match value.as_bool() {
                Some(on) => {
                    if on {
                        confirm = Some(escalation_text());
                    }
                    true
                }
                None => false,
            },
            "headless" => value.is_null() || value.is_boolean(),
            // Typed validation (`AgentConfig`, `WorktreeInput`) already ran; these only need
            // to be present in the allowlist.
            "worktree" => value.is_object() || value.is_null(),
            "maxTurns" | "maxSessionTokens" | "maxPrice" => value.is_null() || value.is_number(),
            "trustWorkspace" => value.is_boolean(),
            other => {
                return Err(policy(
                    method,
                    format!("agentConfig.{other} is not allowed"),
                ));
            }
        };
        if !fine {
            return Err(invalid(
                method,
                format!("agentConfig.{key} has the wrong type or value"),
            ));
        }
    }
    // Every listed root becomes an opted-in project root in Vibe: its hooks, agent profiles,
    // tools, plugins, skills and AGENTS.md load with no trust check (`HarnessFilesManager.
    // for_session` / `project_roots`), so a root is a trust grant and follows the trust rule.
    // Listing the session cwd itself is included: Vibe then loads the untrusted cwd as a root.
    if let Some(roots) = object.get("workspaceRoots").and_then(Value::as_array)
        && !roots
            .iter()
            .all(|root| root.as_str().is_some_and(|r| picked.contains(r)))
    {
        return Err(policy(
            method,
            "workspaceRoots must be inside folders the user picked".to_owned(),
        ));
    }
    if object.get("trustWorkspace").and_then(Value::as_bool) == Some(true) {
        let cwd = object.get("cwd").and_then(Value::as_str);
        if !cwd.is_some_and(|c| picked.contains(c)) {
            return Err(policy(
                method,
                "trustWorkspace needs a cwd inside a folder the user picked".to_owned(),
            ));
        }
        // An existing worktree moves the session there, and Vibe grants the session trust to the
        // worktree path, not to `cwd` (`_worktree_session._rewritten`, then
        // `_build_session_config`). A new worktree (`create`, `auto`) is made by Vibe from the
        // picked repository's own commits, so it is not checked here.
        let worktree = object.get("worktree");
        if worktree.and_then(|w| w.get("kind")).and_then(Value::as_str) == Some("existing") {
            let target = worktree.and_then(|w| w.get("cwd")).and_then(Value::as_str);
            if !target.is_some_and(|t| picked.contains(t)) {
                return Err(policy(
                    method,
                    "trustWorkspace with an existing worktree needs the worktree inside a folder the user picked".to_owned(),
                ));
            }
        }
    }
    if default_headless && object.get("headless").is_none_or(Value::is_null) {
        object.insert("headless".to_owned(), json!(false));
    }
    Ok(Filtered {
        config: Value::Object(object),
        confirm,
    })
}

fn escalation_text() -> String {
    format!(
        "Start with {}: the agent will run every tool without asking",
        vw_vocabulary::label("accessModes", "auto-approve")
    )
}
