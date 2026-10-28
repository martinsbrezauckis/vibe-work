//! One redacted refusal stream for hooks, core tools and browser requests. Called by guards and subscriptions.
//! Uses the existing secret scrubber; keeps the newest 500 decisions per chat for this core run.
use crate::CoreEvent;
use crate::core::Shared;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

/// User-facing guard classes shared by the bell and every refusal producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GuardKind {
    /// A permission refused or left unanswered.
    Ask,
    /// Remote writes, credentials and declined branch rewrites.
    Git,
    /// Session copies and protected application folders.
    Folder,
    /// Browser address and request policy.
    Network,
    /// Foreign process stops and declined kill commands.
    Process,
    /// Built-in or user-supplied safety rules.
    SafetyRule,
    /// A protected-folder review declined by the user.
    ProtectedFolderDeclined,
}

#[derive(Default)]
pub(crate) struct Refusals {
    sequence: AtomicU64,
    chats: Mutex<HashMap<String, VecDeque<CoreEvent>>>,
}
impl Refusals {
    pub(crate) fn events(&self) -> Vec<CoreEvent> {
        let mut events: Vec<_> = self
            .chats
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .flat_map(|chat| chat.iter().cloned())
            .collect();
        events.sort_by_key(|event| match event {
            CoreEvent::GuardRefused { refusal_id, .. } => refusal_id.parse::<u64>().unwrap_or(0),
            _ => 0,
        });
        events
    }
    fn retain(&self, session: &str, event: CoreEvent) -> bool {
        let mut chats = self.chats.lock().unwrap_or_else(PoisonError::into_inner);
        let chat = chats.entry(session.to_owned()).or_default();
        if let CoreEvent::GuardRefused { step_id: Some(step), guard, .. } = &event
            && chat.iter().any(|old| matches!(old, CoreEvent::GuardRefused { step_id: Some(id), guard: kind, .. } if id == step && kind == guard)) {
            return false;
        }
        chat.push_back(event);
        while chat.len() > 500 {
            chat.pop_front();
        }
        true
    }
}

/// Scrub before both the step reply and its bell summary; never copy command inputs into a notice.
pub(crate) fn clean(text: &str) -> String {
    let text = crate::secret_scrub::without_user_info(text);
    vw_secrets::scrub(text.as_bytes()).map_or(text.clone(), |(bytes, _)| {
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// The tool call's history entry when available; a hook may publish before Vibe creates that entry.
pub(crate) fn step(shared: &Shared, session: &str, call: Option<&str>) -> Option<String> {
    let call = call.filter(|id| !id.is_empty());
    let view = shared.store.view(session)?;
    let history = view.state.get("history")?.as_array()?;
    history
        .iter()
        .rev()
        .find(|entry| {
            call.map_or_else(
                || {
                    entry.get("type").and_then(Value::as_str) == Some("effect")
                        && matches!(
                            entry.pointer("/state/status").and_then(Value::as_str),
                            Some("pending" | "running" | "in_progress")
                        )
                },
                |id| {
                    entry.get("id").and_then(Value::as_str) == Some(id)
                        || entry.pointer("/detail/toolCallId").and_then(Value::as_str) == Some(id)
                        || entry.get("toolCallId").and_then(Value::as_str) == Some(id)
                },
            )
        })
        .and_then(|entry| entry.get("id").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| call.map(str::to_owned))
}

/// Publish once at the final refusal boundary. Returns the same scrubbed text for the chat step.
pub(crate) fn record(
    shared: &Shared,
    session: &str,
    guard: GuardKind,
    step_id: Option<String>,
    reason: &str,
) -> String {
    let text = clean(reason);
    if session.is_empty() {
        tracing::warn!("guard refusal has no chat owner");
        return text;
    }
    let what = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let what = if what.chars().count() > 240 {
        format!("{}…", what.chars().take(239).collect::<String>())
    } else {
        what
    };
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let event = CoreEvent::GuardRefused {
        session_id: session.to_owned(),
        guard,
        what,
        step_id,
        at,
        title: clean(&shared.names.display_title(&shared.store, session)),
        refusal_id: (shared
            .guard_refusals
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            + 1)
        .to_string(),
    };
    if shared.guard_refusals.retain(session, event.clone()) {
        shared.emit(event);
    }
    text
}

/// The model proxy owns connector writes; their completed Vibe step supplies chat attribution.
pub(crate) fn observe(shared: &Shared, change: &vw_session::Change) {
    match change {
        vw_session::Change::EntryUpserted {
            session_id, entry, ..
        } => observe_entry(shared, session_id, entry),
        vw_session::Change::EntryPatched {
            session_id,
            entry_id,
            ..
        } => {
            if let Some(view) = shared.store.view(session_id)
                && let Some(entry) = view
                    .state
                    .get("history")
                    .and_then(Value::as_array)
                    .and_then(|history| history.iter().find(|entry| entry["id"] == *entry_id))
            {
                observe_entry(shared, session_id, entry);
            }
        }
        _ => {}
    }
}
fn observe_entry(shared: &Shared, session: &str, entry: &Value) {
    if connector_refused(entry) {
        record(
            shared,
            session,
            GuardKind::Git,
            entry.get("id").and_then(Value::as_str).map(str::to_owned),
            vw_agent_tools::forge::CONNECTOR_REASON,
        );
    }
}
fn connector_refused(entry: &Value) -> bool {
    let tool = entry
        .pointer("/detail/toolName")
        .and_then(Value::as_str)
        .unwrap_or("");
    entry["type"] == "effect"
        && entry.pointer("/state/status").and_then(Value::as_str) == Some("completed")
        && vw_agent_tools::forge::refuses_tool_name(tool) == Some(true)
        && (entry.pointer("/state/outputText").and_then(Value::as_str)
            == Some(vw_agent_tools::forge::CONNECTOR_REASON)
            || entry
                .pointer("/state/output/content")
                .and_then(Value::as_array)
                .is_some_and(|parts| {
                    parts.iter().any(|part| {
                        part.get("text").and_then(Value::as_str)
                            == Some(vw_agent_tools::forge::CONNECTOR_REASON)
                    })
                }))
}

/// Keep folder checks and their step replies on the same refusal boundary.
pub(crate) fn folder<T>(
    shared: &Shared,
    session: &str,
    result: Result<T, String>,
) -> Result<T, String> {
    result.map_err(|reason| {
        record(
            shared,
            session,
            GuardKind::Folder,
            step(shared, session, None),
            &reason,
        )
    })
}

/// Authenticated hook reports cover local decisions as well as core-assisted decisions, exactly once.
pub(crate) fn hook(shared: &Shared, args: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Report {
        session_id: String,
        parent_session_id: Option<String>,
        guard: GuardKind,
        tool_call_id: Option<String>,
        reason: String,
    }
    let report: Report =
        serde_json::from_value(args).map_err(|_| "bad guard refusal report".to_owned())?;
    let session = report
        .parent_session_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or(&report.session_id);
    // A subagent's call id does not identify a step in its parent's transcript.
    let step_id = if session == report.session_id {
        report.tool_call_id.filter(|id| !id.is_empty())
    } else {
        None
    };
    Ok(
        serde_json::json!({ "reason": record(shared, session, report.guard, step_id, &report.reason) }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_completed_forge_step_with_the_proxy_refusal_is_observed() {
        let mut entry = serde_json::json!({"type":"effect", "detail":{"toolName":"connector_github_app.push_files"}, "state":{"status":"completed", "outputText":vw_agent_tools::forge::CONNECTOR_REASON}});
        assert!(connector_refused(&entry));
        entry["state"]["status"] = serde_json::json!("running");
        assert!(!connector_refused(&entry));
        entry["state"]["status"] = serde_json::json!("completed");
        entry["detail"]["toolName"] = serde_json::json!("connector_slack.send_message");
        assert!(!connector_refused(&entry));
    }
    #[test]
    fn retention_is_five_hundred_per_chat_and_keeps_every_guard_kind() {
        let history = Refusals::default();
        for session in ["a", "b"] {
            for n in 0..510 {
                history.retain(
                    session,
                    CoreEvent::GuardRefused {
                        session_id: session.to_owned(),
                        title: session.to_owned(),
                        guard: GuardKind::ProtectedFolderDeclined,
                        what: "The user declined this protected-folder change".to_owned(),
                        step_id: None,
                        at: n,
                        refusal_id: n.to_string(),
                    },
                );
            }
        }
        assert_eq!(history.events().len(), 1000);
        for event in history.events() {
            if let CoreEvent::GuardRefused { at, .. } = event {
                assert!(at >= 10);
            }
        }
        for kind in [
            GuardKind::Git,
            GuardKind::Folder,
            GuardKind::Network,
            GuardKind::Process,
            GuardKind::SafetyRule,
            GuardKind::ProtectedFolderDeclined,
        ] {
            assert_eq!(
                serde_json::from_value::<GuardKind>(serde_json::to_value(kind).unwrap()).unwrap(),
                kind
            );
        }
    }
}
