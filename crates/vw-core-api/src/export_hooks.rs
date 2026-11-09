//! The Markdown export's view of Vibe's structured hook notices: the same rule as the window's transcript
//! (`ui/src/features/transcript/model/hookNotices.ts`), so an export never shows what the chat itself hides.
//! Lifecycle and no-op notices are left out; an outcome is one line; a call a hook refused is written once, as the
//! hook's reason (its skipped step is left out).
//! Depends on: `serde_json`. Called by: `export_md::render`.

use std::collections::{HashMap, HashSet, VecDeque};

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
    if refused_placeholder(effect) {
        return Some(reason);
    }
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

/// A nameless skipped call emitted after a refusal by newer Vibe versions.
fn refused_placeholder(entry: &Value) -> bool {
    entry["type"] == "effect"
        && entry["state"]["status"] == "skipped"
        && !entry["detail"]["input"].is_object()
        && matches!(text(&entry["detail"], "toolName"), "" | "tool")
}

/// Pair explicit call ids first, then nameless placeholders in refusal order within a user-message boundary.
fn refused_steps(entries: &[Value]) -> HashMap<&str, &Value> {
    let by_id: HashMap<_, _> = entries
        .iter()
        .map(|entry| (text(entry, "id"), entry))
        .collect();
    let mut steps = HashMap::new();
    let mut waiting: VecDeque<(&str, Vec<&str>)> = VecDeque::new();
    for entry in entries {
        let detail = &entry["detail"];
        if entry["type"] == "message" && entry["role"] == "user" {
            waiting.clear();
        } else if is_hook_notice(entry)
            && detail["kind"] == "hook_completed"
            && detail["scope"] == "pre_tool"
        {
            let call = text(detail, "toolCallId");
            if let Some(named) = by_id.get(call).filter(|entry| entry["type"] == "effect") {
                steps.insert(text(entry, "id"), *named);
            } else if !call.is_empty() && detail["status"] == "error" {
                if let Some((_, notices)) = waiting.iter_mut().find(|(id, _)| *id == call) {
                    notices.push(text(entry, "id"));
                } else {
                    waiting.push_back((call, vec![text(entry, "id")]));
                }
            }
        } else if refused_placeholder(entry)
            && let Some((_, notices)) = waiting.pop_front()
        {
            for notice in notices {
                steps.insert(notice, entry);
            }
        }
    }
    steps
}

/// One projection for the export's hook lines and suppressed steps; raw history stays unchanged.
pub(crate) struct Projection<'a> {
    pub(crate) lines: HashMap<&'a str, String>,
    pub(crate) refused: HashSet<&'a str>,
}

/// The shared fixture contract also runs against the transcript's projection.
pub(crate) fn project(entries: &[Value]) -> Projection<'_> {
    let steps = refused_steps(entries);
    let mut projected = Projection {
        lines: HashMap::new(),
        refused: HashSet::new(),
    };
    for entry in entries.iter().filter(|entry| is_hook_notice(entry)) {
        let id = text(entry, "id");
        if let Some(effect) = steps.get(id)
            && denial_reason(Some(effect), text(&entry["detail"], "hookName")).is_some()
        {
            projected.refused.insert(text(effect, "id"));
        }
        if let Some(line) = line(entry, &steps) {
            projected.lines.insert(id, line);
        }
    }
    projected
}

/// Plain action labels match the transcript's builtin-tool permissions, never a custom tool's guessed identity.
fn tool_permission(detail: &Value, content: &str) -> &'static str {
    let tool = Some(text(detail, "toolName"))
        .filter(|tool| !tool.is_empty())
        .or_else(|| {
            content
                .strip_prefix("Denied tool '")
                .and_then(|tool| tool.strip_suffix("'"))
        })
        .unwrap_or_default();
    let tool = tool
        .strip_prefix("file_system.")
        .or_else(|| tool.strip_prefix("ui."))
        .unwrap_or(tool);
    match tool {
        "bash" | "shell" => "running commands",
        "read_file" => "reading files",
        "write_file" => "creating and changing files",
        "edit" | "search_replace" => "changing files",
        "grep" => "searching file contents",
        "glob" => "finding files",
        "list_directory" => "listing folders",
        "web.fetch" | "web_fetch" => "reading web pages",
        "web.search" | "web_search" => "searching the web",
        "skill" | "skill.read" | "read_skill" => "reading skill instructions",
        "task.create" | "task.update" | "task.list" | "task.get" => "managing tasks",
        "todo" => "updating the task list",
        "task" | "agents.spawn" | "subagent.spawn" | "agent" | "agent.run" => "running a subagent",
        "self.wait" | "wait" | "self.sleep" => "waiting",
        "agents.list_configs" => "listing agent configurations",
        "agents.list" | "subagent.list" => "listing subagents",
        "agents.send_message" | "subagent.send_message" => "sending messages to subagents",
        "agents.interrupt" | "subagent.interrupt" => "interrupting subagents",
        "subagent.stop" => "stopping subagents",
        "subagent.wait" | "agent.wait" => "waiting for subagents",
        "ask_user_question" | "ask_user" | "user_input" => "asking a question",
        "exit_plan_mode" => "finishing a plan",
        "mcp_vw.session_status"
        | "mcp_vw.connectors_list"
        | "mcp_vw.connector_suggest"
        | "mcp_vw.addon_suggest"
        | "mcp_vw.document_read"
        | "mcp_vw.propose_protected_change"
        | "mcp_vw.protected_change_status"
        | "mcp_vw.read_aloud"
        | "mcp_vw.processes_list"
        | "mcp_vw.port_lease"
        | "mcp_vw.process_start"
        | "mcp_vw.process_stop"
        | "mcp_vw.process_restart"
        | "mcp_vw.use_headless_browser"
        | "mcp_vw.secret_list"
        | "mcp_vw.secret_run"
        | "mcp_vw.secret_run_status"
        | "mcp_vw.panel_read"
        | "mcp_vw.preview_list"
        | "mcp_vw.preview_open"
        | "mcp_vw.preview_screenshot"
        | "mcp_vw.preview_console"
        | "mcp_vw.preview_close"
        | "mcp_vw.browser_tabs"
        | "mcp_vw.browser_open"
        | "mcp_vw.browser_read"
        | "mcp_vw.browser_act"
        | "mcp_vw.browser_screenshot"
        | "mcp_vw.browser_close"
        | "mcp_vw.browser_fill_login"
        | "mcp_vw.browser_check_view"
        | "mcp_vw.browser_capture_secret"
        | "mcp_vw.chrome_tool_list"
        | "mcp_vw.browser_capture"
        | "mcp_vw.chrome_status"
        | "mcp_vw.chrome_session"
        | "mcp_vw.chrome_execute" => "this action",
        _ => "the call",
    }
}

/// The line a hook notice is written as, or `None` when the window leaves it out.
pub(crate) fn line(entry: &Value, steps: &HashMap<&str, &Value>) -> Option<String> {
    let detail = &entry["detail"];
    if detail["kind"] != "hook_completed" {
        return None;
    }
    let content = text(detail, "content");
    let status = text(detail, "status");
    let name = Some(text(detail, "hookName"))
        .filter(|n| !n.is_empty())
        .unwrap_or("Hook");
    let effect = steps.get(text(entry, "id")).copied();
    let pre_tool = detail["scope"] == "pre_tool";
    let reason = if pre_tool {
        denial_reason(effect, name)
    } else {
        None
    };
    let refused = || {
        format!(
            "Not run: {name} refused {}.",
            tool_permission(detail, content)
        )
    };
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
    if reason.is_none()
        && ((name == "vibe-work tool receipt" && status == "ok")
            || (matches!(
                name,
                "vibe-work safety reminder" | "vibe-work tool receipt" | "vibe-work folder guard"
            ) && pre_tool
                && status == "warning"
                && input_rewrite(content)))
    {
        return None;
    }
    // The decision note is appended for the agent; the person already made that decision.
    if name == "vibe-work allowed note"
        && detail["scope"] == "post_tool"
        && status == "warning"
        && content
            .strip_prefix("Appended ")
            .and_then(|rest| rest.strip_suffix(" chars to tool result"))
            .is_some_and(|count| {
                !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit())
            })
    {
        return None;
    }
    if let Some(reason) = reason {
        return Some(if reason.is_empty() {
            format!("Not run: {name} refused the call.")
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn folder_rewrites_stay_out_of_exported_conversation() {
        for tool in ["read", "edit", "bash"] {
            let entry = json!({"type":"notice", "detail": {
                "kind":"hook_completed", "scope":"pre_tool", "hookName":"vibe-work folder guard",
                "status":"warning", "content":format!("Rewrote tool_input for '{tool}'")
            }});
            assert_eq!(line(&entry, &HashMap::new()), None);
        }
    }

    #[test]
    fn folder_refusals_and_unfamiliar_warnings_remain_exported() {
        for (status, content) in [
            ("error", "Folder unavailable"),
            ("warning", "Folder changed"),
        ] {
            let entry = json!({"type":"notice", "detail": {
                "kind":"hook_completed", "scope":"pre_tool", "hookName":"vibe-work folder guard",
                "status":status, "content":content
            }});
            assert!(line(&entry, &HashMap::new()).is_some());
        }
    }
}
