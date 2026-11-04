//! App-owned tool cards wait in the authenticated pre-tool hook.
//! The callback adapter answers Vibe's transport gate after that decision; tools consume it once.
use crate::{CommandResult, CoreError, CoreEvent, ErrorCode, Origin, core::Shared};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use vw_agent_tools::wire::tools as names;
use vw_ask::{AnsweredBy, Outcome, Scope};

const PREFIX: &str = "vw-tool-";
mod read_rules;
pub(crate) mod receipts;
struct Call {
    session: String,
    entry: Value,
}
#[derive(Default)]
pub(crate) struct Calls {
    records: Mutex<VecDeque<Call>>,
    transports: Mutex<VecDeque<receipts::Receipt>>,
}
fn calls(shared: &Shared) -> Option<&Calls> {
    shared.agent_tools.as_ref().map(|t| &t.tool_asks)
}
fn lock(calls: &Calls) -> std::sync::MutexGuard<'_, VecDeque<Call>> {
    calls.records.lock().unwrap_or_else(PoisonError::into_inner)
}
pub(crate) fn local(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// Tools whose own policy decides at hook time, without the generic access-mode card.
fn asks_inside(tool: &str, args: &Value) -> bool {
    matches!(
        tool,
        names::SECRET_RUN
            | names::SESSION_STATUS
            | names::CONNECTORS_LIST
            | names::CONNECTOR_SUGGEST
            | names::ADDON_SUGGEST
            | names::SECRET_RUN_STATUS
            | names::PROPOSE_PROTECTED_CHANGE
            | names::PROTECTED_CHANGE_STATUS
            | names::BROWSER_CAPTURE_SECRET
            | names::DOCUMENT_READ
            | names::READ_ALOUD
            | names::USE_HEADLESS_BROWSER
    ) || names::is_chrome(tool)
        || (tool == names::BROWSER_ACT
            && matches!(args["action"].as_str(), Some("fill" | "close_tab")))
}

/// Calls that only read what this chat can already see (its preview list and console, a picture of its preview, its
/// folders). In Accept edits they run without a card, as reading does in other coding agents; the other modes keep the
/// card. Opening, clicking, typing, every write and every call that could change what is shown
/// keep their approval: a preview picture that sets a width, reading a browser page (it can open, switch or reload a
/// tab) and reading the side panels (an editor there can hold a file the person's read rules exclude).
fn reads_only(tool: &str, args: &Value) -> bool {
    match tool {
        names::PREVIEW_LIST | names::PREVIEW_CONSOLE | names::SEARCH => true,
        names::PREVIEW_SCREENSHOT => args.get("width").is_none_or(Value::is_null),
        _ => false,
    }
}

/// Whether the person set a stricter rule for `tool` in Vibe (`read_rules`); only the default card goes.
async fn stricter_rule(shared: &Arc<Shared>, owner: &str, tool: &str) -> bool {
    let home = shared.instructions.own_home.clone();
    // The chat's folders as its tools see them; read without the folder guard's own refusal record (the tool's
    // run still checks them).
    let roots: Vec<std::path::PathBuf> = crate::processes::contexts(&shared.store)
        .into_iter()
        .find(|c| c.session_id == owner)
        .map(|c| c.folders)
        .unwrap_or_default();
    let tool = tool.to_owned();
    tokio::task::spawn_blocking(move || read_rules::stricter(&home, &roots, "accept-edits", &tool))
        .await
        .unwrap_or(true)
}

pub(crate) async fn permission(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    let mode = shared.access.remembered(owner);
    if asks_inside(tool, args) || mode.as_deref() == Some("auto-approve") {
        return Ok(());
    }
    if reads_only(tool, args)
        && mode.as_deref() == Some("accept-edits")
        && !stricter_rule(shared, owner, tool).await
    {
        return Ok(());
    }
    let detail = json!({"kind":"approval", "effect":{"kind":"tool", "toolName":format!("mcp_vw.{tool}"), "input":args}, "requiredPermissions":[]});
    #[cfg(feature = "video")]
    let detail = if tool == "video_frames" {
        let request: vw_agent_tools::video::Request =
            serde_json::from_value(args.clone()).map_err(|e| format!("Invalid video request: {e}. Supply integer millisecond times, a supported selection and output mode."))?;
        let mut detail = detail;
        detail["effect"]["display"] = json!({"summary":request.preflight()?});
        detail
    } else {
        detail
    };
    approve(shared, owner, source, call, detail).await
}

pub(crate) async fn approve(
    shared: &Arc<Shared>,
    session: &str,
    source: &str,
    call: &str,
    detail: Value,
) -> Result<(), String> {
    let id = format!(
        "{PREFIX}{}",
        vw_agent_tools::credential::new_credential().map_err(|e| e.to_string())?
    );
    let mut entry = json!({"id":id, "sessionId":session, "callbackId":id,
        "type":"callback", "createdAt":shared.asks.now_ms(), "detail":detail,
        "vwSourceCallId":call, "vwToolAsk":true, "state":{"status":"open"}});
    if let Some(view) = shared.store.view(source) {
        entry["turnId"] = view.state["session"]["status"]["activeTurnId"].clone();
    }
    if source != session {
        entry["vwSourceSession"] = json!(source);
    }
    crate::chrome_control::approval::project(shared, &mut entry);
    if let Some(grant) = shared.ask_grants.matching(session, &entry) {
        shared
            .ask_grants
            .record_use(&grant.id, shared.asks.now_ms());
        shared.emit(super::grants::event(shared));
        carry_over(shared, session, &id, &entry);
        return Ok(());
    }
    let stop = crate::stops::generation(shared, session);
    {
        let mut records = lock(calls(shared).ok_or("Tools unavailable.")?);
        make_room(shared, &mut records)?;
        records.push_back(Call {
            session: session.into(),
            entry: entry.clone(),
        });
    }
    let mut handle = super::callback_opened_at(shared, &entry, "approval", session, Some(stop))
        .ok_or("Permission prompt unavailable.")?;
    handle.cancel_on_drop = true;
    shared.emit(CoreEvent::CallbackOpened {
        session_id: session.into(),
        callback_id: id.clone(),
        kind: "approval".into(),
        subagent: None,
    });
    changed(shared, session);
    let outcome = handle.closed().await;
    changed(shared, session);
    if !matches!(outcome, Outcome::Allowed { .. }) {
        return Err(vw_ask::agent_text(&outcome, "this tool call"));
    }
    if crate::stops::generation(shared, session) != stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    carry_over(shared, session, &id, &entry);
    Ok(())
}

fn carry_over(shared: &Shared, session: &str, id: &str, entry: &Value) {
    if let Some(ledger) = crate::one_ask::of(shared) {
        ledger.card_opened(entry);
        ledger.card_answered(session, id, &crate::one_ask::approve_once(), Origin::Ui);
    }
}

/// Only the authenticated hook can approve a call. Children inherit it in pinned Vibe.
pub(crate) fn dispatch_transport(shared: &Arc<Shared>, entry: &Value, owner: &str) -> bool {
    let Some(tool) = entry["detail"]["effect"]["toolName"]
        .as_str()
        .and_then(|n| n.strip_prefix("mcp_vw."))
    else {
        return false;
    };
    if entry["detail"]["kind"] != "approval" {
        return false;
    }
    let result = calls(shared)
        .ok_or("Tools unavailable.".to_owned())
        .and_then(|calls| {
            receipts::verify(
                calls,
                owner,
                entry["sessionId"].as_str().unwrap_or_default(),
                entry["id"]
                    .as_str()
                    .unwrap_or_default()
                    .strip_prefix("approval-")
                    .unwrap_or_default(),
                tool,
                &entry["detail"]["effect"]["input"],
            )
        });
    let output = match result {
        Ok(()) => json!({"type":"approval","decision":{"type":"approve"}}),
        Err(reason) => json!({"type":"approval","decision":{"type":"deny"},"feedback":reason}),
    };
    let shared = shared.clone();
    let session = entry["sessionId"].as_str().unwrap_or_default().to_owned();
    let id = entry["id"].as_str().unwrap_or_default().to_owned();
    tokio::spawn(async move {
        if let Err(problem) = shared.gate.answer_callback(&session, &id, output).await {
            tracing::warn!(reason = %problem, "MCP transport approval not confirmed");
        }
    });
    true
}

/// The existing card answer route, scoped to the app-owned card and its actual session.
pub(crate) fn answer_by(
    shared: &Arc<Shared>,
    origin: Origin,
    session: &str,
    id: &str,
    output: &Value,
    by: AnsweredBy,
) -> Result<CommandResult, CoreError> {
    let refused = || {
        CoreError::new(
            ErrorCode::CallbackRefused,
            "This permission is no longer open.",
        )
    };
    let Some(calls) = calls(shared) else {
        return Err(refused());
    };
    if !lock(calls)
        .iter()
        .any(|c| c.session == session && c.entry["id"] == id)
    {
        return Err(refused());
    }
    let ask_id = format!("callback:{id}");
    let scope = match output["decision"]["type"].as_str() {
        Some("approve") => Some(Scope::Once),
        Some("approve_for_session")
            if shared
                .asks
                .get(&ask_id)
                .is_some_and(|a| a.offers.contains(&vw_ask::Choice::Session)) =>
        {
            Some(Scope::Session)
        }
        Some("deny" | "cancel_turn") => None,
        _ => {
            return Err(CoreError::new(
                ErrorCode::CallbackRefused,
                "This answer is not offered.",
            ));
        }
    };
    let outcome = if output["decision"]["type"] == "cancel_turn" {
        Outcome::Cancelled {
            reason: "the turn was stopped".into(),
        }
    } else {
        scope.map_or_else(
            || {
                super::refused(
                    by,
                    output["feedback"].as_str().map(super::service::scrub_note),
                )
            },
            |scope| super::allowed(by, scope),
        )
    };
    let delivery = shared.asks.reserve(&ask_id, by).ok_or_else(refused)?;
    // A No releases the held hook with the declined result (`approve`); the turn goes on.
    // Publish attribution before the receipt can make an audit row visible.
    shared.emit(CoreEvent::CallbackAnswered {
        session_id: session.into(),
        callback_id: id.into(),
        origin,
    });
    delivery.confirmed(outcome);
    if scope == Some(Scope::Session)
        && let Some(pending) = shared.ask_grants.claim(id)
        && shared
            .ask_grants
            .keep(pending, by, shared.asks.now_ms())
            .is_some()
    {
        shared.emit(super::grants::event(shared));
    }
    shared.ask_grants.forget_callback(id);
    changed(shared, session);
    Ok(CommandResult::Reply {
        result: Value::Null,
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
    })
}

pub(crate) fn overlay(shared: &Shared, session: &str, state: &mut Value) {
    let Some(calls) = calls(shared) else { return };
    let Some(history) = state["history"].as_array_mut() else {
        return;
    };
    history.retain(|entry| entry["vwToolAsk"] != true);
    for call in lock(calls).iter().filter(|c| c.session == session) {
        let mut entry = call.entry.clone();
        let id = entry["id"].as_str().unwrap_or_default();
        if let Some(record) = shared.asks.outcome(&format!("callback:{id}")) {
            entry["state"] = json!({"status":"answered","output":{"type":"approval","decision":{"type":if matches!(record.outcome, Outcome::Allowed{..}) {"approve"} else {"deny"}}}});
        }
        let next = history
            .iter()
            .position(|e| e["createdAt"].as_u64() > entry["createdAt"].as_u64());
        history.insert(next.unwrap_or(history.len()), entry);
    }
}
pub(crate) fn changed(shared: &Shared, session: &str) {
    if let Ok(snapshot) = crate::sessions::snapshot(shared, session) {
        shared.emit(CoreEvent::Snapshot { snapshot });
    }
}

/// App-owned cards are also visible through the existing callback listing used by the Debug API.
pub(crate) fn open(shared: &Shared) -> Vec<crate::CallbackInfo> {
    let Some(calls) = calls(shared) else {
        return Vec::new();
    };
    lock(calls)
        .iter()
        .filter_map(|c| {
            let id = c.entry["id"].as_str()?;
            (shared.asks.get(&format!("callback:{id}")).is_some()).then(|| crate::CallbackInfo {
                callback_id: id.into(),
                session_id: c.session.clone(),
                kind: "approval".into(),
            })
        })
        .collect()
}

fn make_room(shared: &Shared, records: &mut VecDeque<Call>) -> Result<(), String> {
    while records.len() >= 500 {
        let index = records
            .iter()
            .position(|c| {
                shared
                    .asks
                    .get(&format!(
                        "callback:{}",
                        c.entry["id"].as_str().unwrap_or_default()
                    ))
                    .is_none()
            })
            .ok_or("Too many open permission prompts.")?;
        records.remove(index);
    }
    Ok(())
}
#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn tools_with_their_own_policy_never_get_a_generic_card() {
        for name in [
            names::DOCUMENT_READ,
            names::SECRET_RUN,
            names::SECRET_RUN_STATUS,
            names::PROPOSE_PROTECTED_CHANGE,
            names::PROTECTED_CHANGE_STATUS,
            names::BROWSER_CAPTURE_SECRET,
            names::CHROME_TOOL_LIST,
            "chrome_status",
            "chrome_session",
            "chrome_execute",
        ] {
            assert!(asks_inside(name, &json!({})), "{name}");
        }
        for action in ["fill", "close_tab"] {
            assert!(asks_inside(names::BROWSER_ACT, &json!({"action":action})));
        }
        for name in [names::READ_ALOUD, names::USE_HEADLESS_BROWSER] {
            let card = json!({"detail":{"kind":"approval","effect":{"toolName":format!("mcp_vw.{name}")}}});
            assert!(
                crate::read_aloud::approve_without_dialog(&card)
                    || crate::on_demand::approve_without_dialog(&card),
                "{name}"
            );
        }
        assert!(!asks_inside(
            names::PROCESS_START,
            &json!({"command":"echo hello"})
        ));
        assert!(!asks_inside(names::BROWSER_ACT, &json!({"action":"click"})));
    }
}

#[cfg(test)]
#[path = "tool_calls_cap_tests.rs"]
mod cap_tests;

#[cfg(test)]
#[path = "tool_calls_read_tests.rs"]
mod read_tests;

#[cfg(test)]
mod suggestion_tests {
    use super::*;
    #[tokio::test]
    async fn addon_cards_need_no_approval_in_any_access_mode() {
        let core = crate::offline_core::start("addon-card-permission").await;
        for mode in [
            "ask",
            "accept-edits",
            "smart-approve",
            "auto-approve",
            "vw-plan",
            "plan",
            "explore",
        ] {
            core.shared.access.observe_request(
                "session/agent/update",
                "s",
                &json!({"agentName":mode}),
            );
            for tool in [names::CONNECTOR_SUGGEST, names::ADDON_SUGGEST] {
                let answer = tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    permission(
                        &core.shared,
                        "s",
                        "s",
                        "call",
                        tool,
                        &json!({"addon":"chrome","reason":"Read a page"}),
                    ),
                )
                .await;
                assert!(matches!(answer, Ok(Ok(()))), "{mode} {tool}: {answer:?}");
                assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
            }
        }
        core.shutdown().await;
    }
}
