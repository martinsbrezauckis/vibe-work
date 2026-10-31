//! The Markdown export's view of Vibe's structured hook notices: the same rule as the window's transcript
//! (`ui/src/features/transcript/model/hookNotices.ts`), so an export never shows what the chat itself hides.
//! Lifecycle and no-op notices are left out; an outcome is one line; a call a hook refused is written once, as the
//! hook's reason (its skipped step is left out).
//! Depends on: `serde_json`. Called by: `export_md::render`.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

const HOOK_KINDS: &[&str] = &[
    "hook_run_started",
    "hook_run_completed",
    "hook_started",
    "hook_completed",
];

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A structured hook notice, never inferred from the human-readable message.
pub(crate) fn is_hook_notice(entry: &Value) -> bool {
    entry["type"] == "notice" && HOOK_KINDS.contains(&text(&entry["detail"], "kind"))
}

/// The exact reason Vibe wraps around a skipped hook-denied call, without its internal markup.
fn denial_reason<'a>(effect: Option<&'a Value>, hook: &str) -> Option<&'a str> {
    let effect = effect.filter(|e| e["type"] == "effect" && e["state"]["status"] == "skipped")?;
    let reason = text(&effect["state"], "reason");
    let tool = text(&effect["detail"], "toolName");
    reason
        .strip_prefix(&format!(
            "<tool_error>Tool '{tool}' was denied by hook '{hook}': "
        ))?
        .strip_suffix("</tool_error>")
}

/// `Rewrote tool_input for '<tool>'` with a tool name of `[a-z][a-z0-9_]*`.
fn input_rewrite(content: &str) -> bool {
    content
        .strip_prefix("Rewrote tool_input for '")
        .and_then(|rest| rest.strip_suffix('\''))
        .is_some_and(|tool| {
            tool.starts_with(|c: char| c.is_ascii_lowercase())
                && tool
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// History entries by id, for [`line`] and [`refused_calls`].
pub(crate) fn by_id(entries: &[Value]) -> HashMap<&str, &Value> {
    entries
        .iter()
        .filter_map(|e| e.get("id").and_then(Value::as_str).map(|id| (id, e)))
        .collect()
}

/// The line a hook notice is written as, or `None` when the window leaves it out.
pub(crate) fn line(entry: &Value, by_id: &HashMap<&str, &Value>) -> Option<String> {
    let detail = &entry["detail"];
    if detail["kind"] != "hook_completed" {
        return None;
    }
    let content = text(detail, "content");
    let status = text(detail, "status");
    let name = Some(text(detail, "hookName"))
        .filter(|n| !n.is_empty())
        .unwrap_or("Hook");
    let effect = by_id.get(text(detail, "toolCallId")).copied();
    let pre_tool = detail["scope"] == "pre_tool";
    let reason = if pre_tool {
        denial_reason(effect, name)
    } else {
        None
    };
    let refused = || format!("Not run: {name} refused the call.");
    if pre_tool && status == "error" {
        if content.ends_with(" (strict)") {
            return Some(format!("{name} failed; the call was not run."));
        }
        return Some(match reason {
            Some(reason) if !reason.is_empty() => reason.to_owned(),
            _ if content.starts_with("Denied tool '") || content.is_empty() => refused(),
            _ => content.to_owned(),
        });
    }
    // A successful transport binding or input rewrite is bookkeeping, not conversation.
    if name == "vibe-work tool receipt"
        && reason.is_none()
        && (status == "ok" || (pre_tool && status == "warning" && input_rewrite(content)))
    {
        return None;
    }
    if let Some(reason) = reason {
        return Some(if reason.is_empty() {
            refused()
        } else {
            reason.to_owned()
        });
    }
    if !content.trim().is_empty() {
        return Some(format!("{name}: {content}"));
    }
    if status == "ok" {
        return None;
    }
    let message = Some(text(entry, "message"))
        .filter(|m| !m.is_empty())
        .unwrap_or(if status == "error" {
            "failed"
        } else {
            "reported a warning"
        });
    Some(format!("{name}: {message}"))
}

/// The tool calls a hook refused whose refusal its notice already says (their skipped steps are left out).
pub(crate) fn refused_calls<'a>(
    entries: &'a [Value],
    by_id: &HashMap<&str, &Value>,
) -> HashSet<&'a str> {
    entries
        .iter()
        .filter(|e| is_hook_notice(e) && e["detail"]["kind"] == "hook_completed")
        .filter(|e| e["detail"]["scope"] == "pre_tool")
        .filter_map(|e| {
            let call = text(&e["detail"], "toolCallId");
            denial_reason(by_id.get(call).copied(), text(&e["detail"], "hookName")).map(|_| call)
        })
        .collect()
}
