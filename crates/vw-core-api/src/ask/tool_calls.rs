//! App-owned tool cards wait in the authenticated pre-tool hook.
//! The callback adapter answers Vibe's transport gate after that decision; tools consume it once.
use crate::{CommandResult, CoreError, CoreEvent, ErrorCode, Origin, core::Shared};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use vw_agent_tools::wire::tools as names;
use vw_ask::{AnsweredBy, Outcome, Scope};

const PREFIX: &str = "vw-tool-";
mod announced;
mod browser_requests;
mod read_rules;
pub(crate) mod receipts;
#[cfg(feature = "agent-vault")]
pub(crate) mod vault_save;
struct Call {
    session: String,
    entry: Value,
}
#[derive(Default)]
pub(crate) struct Calls {
    records: Mutex<VecDeque<Call>>,
    /// The profile the person picked on a browser profile card, by card id, until the held call takes it.
    picked: Mutex<std::collections::BTreeMap<String, String>>,
    /// The turn in which the person said No on a chat's browser profile card, by chat: that turn shows no other
    /// profile card (`choose_profile`).
    declined: Mutex<std::collections::BTreeMap<String, String>>,
    transports: Mutex<VecDeque<receipts::Receipt>>,
    /// Woken when the core sees Vibe carry a receipt into a call (`receipts::observe`).
    announced: tokio::sync::Notify,
    pub(crate) browser_requests: Arc<browser_requests::Ledger>,
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
    ) || (cfg!(feature = "agent-vault") && tool == names::SECRET_CAPTURE_COMMAND)
        || names::is_chrome(tool)
        || (tool == names::BROWSER_ACT
            && matches!(args["action"].as_str(), Some("fill" | "close_tab")))
}

/// Tools that skip the default Accept edits card. Explicit read and tool rules still apply.
/// Port/process queries retain their internal bookkeeping; browser reads may open, front or reload a page,
/// and captures save output. Preview resizing and handbook repairs keep their card.
fn accepts_edits(tool: &str, args: &Value) -> bool {
    match tool {
        names::PREVIEW_LIST
        | names::PREVIEW_CONSOLE
        | names::SEARCH
        | names::PROCESSES_LIST
        | names::PORT_LEASE
        | names::PANEL_READ
        | names::HANDBOOK_ANSWERS
        | names::BROWSER_READ
        | names::BROWSER_CAPTURE => true,
        names::HANDBOOK_CHECK => args.get("repair").is_none_or(Value::is_null),
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
    // An unavailable tool returns its reason through the call, without offering an approval card.
    if tool == "video_frames" && !vw_agent_tools::video::ENABLED {
        return Ok(());
    }
    if matches!(
        tool,
        names::BROWSER_ACT | names::BROWSER_READ | names::BROWSER_CAPTURE
    ) {
        crate::browser_handover::check(shared, owner)?;
    }
    let mode = shared.access.remembered(owner);
    if tool == names::BROWSER_ACT {
        vw_agent_tools::browser_input::validate(tool, args)?;
        crate::chrome_control::builtin::check_full(mode.as_deref(), args)?;
        // The authenticated hook settles the ordinary policy separately for every step.
        if args.get("steps").is_some() {
            return Ok(());
        }
    }
    let ask = mode.as_deref() == Some("ask");
    let browser_tier = (tool == names::BROWSER_ACT)
        .then(|| crate::chrome_control::builtin::approval_tier(mode.as_deref(), args))
        .flatten();
    let browser_write = browser_tier.is_some();
    let headless = ask && tool == names::USE_HEADLESS_BROWSER;
    if headless
        && shared
            .agent_tools
            .as_ref()
            .is_some_and(|tools| tools.on_demand.consented(owner))
    {
        return Ok(());
    }
    if ask
        && (matches!(tool, names::BROWSER_READ | names::BROWSER_CAPTURE)
            || tool == names::BROWSER_ACT
                && crate::chrome_control::builtin::ask_required(args)
                    == Some(vw_ask::chrome::Tier::Ask))
        && !stricter_rule(shared, owner, tool).await
    {
        return Ok(());
    }
    // Several connected profiles and no default: the person picks this chat's profile before the open runs. The
    // pick waits here, in the authenticated hook, like every person card, so no tool timeout cuts it short.
    if tool == "chrome_session" && args["action"] == "open" {
        crate::chrome_control::ask_profile(shared, owner, args).await?;
    }
    if (asks_inside(tool, args) && !browser_write && !headless)
        || mode.as_deref() == Some("auto-approve")
    {
        return Ok(());
    }
    if (accepts_edits(tool, args)
        || (tool == names::BROWSER_ACT && crate::chrome_control::builtin::accepts_edits(args)))
        && (mode.as_deref() == Some("accept-edits")
            || tool == names::BROWSER_ACT && mode.as_deref() == Some("smart-approve"))
        && !stricter_rule(shared, owner, tool).await
    {
        return Ok(());
    }
    let mut detail = json!({"kind":"approval", "effect":{"kind":"tool", "toolName":format!("mcp_vw.{tool}"), "input":args}, "requiredPermissions":[]});
    if let Some(tier) = browser_tier {
        detail["vwBrowserTier"] = json!(tier as u8);
    }
    #[cfg(feature = "video")]
    let detail = if tool == "video_frames" {
        let request: vw_agent_tools::video::Request = serde_json::from_value(args.clone())
            .map_err(|e| {
                crate::video_projection::failure(&vw_video_types::Error::new(
                    vw_video_types::Code::InvalidRequest,
                    e,
                ))
                .1
            })?;
        let mut detail = detail;
        detail["effect"]["display"] = json!({"summary":request.preflight().map_err(|e| crate::video_projection::failure(&e).1)?});
        detail
    } else {
        detail
    };
    approve(shared, owner, source, call, detail).await?;
    if headless && let Some(tools) = &shared.agent_tools {
        tools.on_demand.allow(owner);
    }
    Ok(())
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
        let mut records = lock(calls(shared).ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE)?);
        make_room(shared, &mut records)?;
        records.push_back(Call {
            session: session.into(),
            entry: entry.clone(),
        });
    }
    let mut handle = super::callback_opened_at(shared, &entry, "approval", session, Some(stop))
        .ok_or(vw_vocabulary::texts::AGENT_PROMPT_UNAVAILABLE)?;
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
    if let Some(ledger) = crate::one_ask::of(shared) {
        ledger.note_allowed(&[source, session], call);
    }
    carry_over(shared, session, &id, &entry);
    Ok(())
}

/// One profile offered on a browser profile card.
pub(crate) struct ProfileChoice {
    /// Exact paired identity.
    pub(crate) profile_id: String,
    /// The browser's own profile name, shown on the card.
    pub(crate) name: String,
    /// The profile's name in agent text: the browser's name with the account's domain, never the account.
    pub(crate) agent_name: String,
    /// The signed-in account, shown under the name on the card only.
    pub(crate) account: Option<String>,
}

/// Which profile should this chat use: the person picks one of `choices` on a card in the chat (`browser` names
/// it). Holds the agent's call until the answer and returns the picked identity; a No, a Stop or an unavailable
/// card returns the agent's next step. The card lists the account for the person; the agent only gets the result.
/// `closed` adds the card's note about paired profiles it cannot offer (`closedDefault` / `closedOthers`); once
/// answered, the card's effect keeps the picked identity (`picked`) so its audit row can name the profile.
pub(crate) async fn choose_profile(
    shared: &Arc<Shared>,
    session: &str,
    browser: &str,
    choices: &[ProfileChoice],
    closed: &Value,
) -> Result<String, String> {
    // After a No, the same turn gets the same answer without another card.
    let turn = active_turn(shared, session);
    if let (Some(calls), Some(turn)) = (calls(shared), turn.as_deref())
        && declined_turn(calls, session).as_deref() == Some(turn)
    {
        return Err(profile_declined(browser));
    }
    let id = format!(
        "{PREFIX}{}",
        vw_agent_tools::credential::new_credential().map_err(|e| e.to_string())?
    );
    let rows: Vec<Value> = choices
        .iter()
        .map(|choice| json!({"profileId":choice.profile_id, "name":choice.name, "account":choice.account}))
        .collect();
    let mut detail = json!({"kind":"approval", "requiredPermissions":[],
        "effect":{"kind":"browserProfile", "browser":browser, "choices":rows}});
    if let (Some(effect), Some(note)) = (detail["effect"].as_object_mut(), closed.as_object()) {
        effect.extend(note.clone());
    }
    let mut entry = json!({"id":id, "sessionId":session, "callbackId":id,
        "type":"callback", "createdAt":shared.asks.now_ms(), "detail":detail,
        "vwToolAsk":true, "state":{"status":"open"}});
    if let Some(view) = shared.store.view(session) {
        entry["turnId"] = view.state["session"]["status"]["activeTurnId"].clone();
    }
    let names: Vec<&str> = choices
        .iter()
        .map(|choice| choice.agent_name.as_str())
        .collect();
    let unavailable = || {
        format!(
            "Several {browser} profiles are connected ({}) and the profile question could not be shown; no action ran. Ask the user to choose a default profile in Settings > Browser.",
            names.join(", ")
        )
    };
    let stop = crate::stops::generation(shared, session);
    let calls = calls(shared).ok_or_else(unavailable)?;
    {
        let mut records = lock(calls);
        make_room(shared, &mut records).map_err(|_| unavailable())?;
        records.push_back(Call {
            session: session.into(),
            entry: entry.clone(),
        });
    }
    let mut handle = super::callback_opened_at(shared, &entry, "approval", session, Some(stop))
        .ok_or_else(unavailable)?;
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
    let picked = calls
        .picked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
    if matches!(outcome, Outcome::Refused { .. }) {
        if let Some(turn) = turn {
            calls
                .declined
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(session.into(), turn);
        }
        return Err(profile_declined(browser));
    }
    if !matches!(outcome, Outcome::Allowed { .. }) {
        return Err(format!(
            "{} No action ran. The user can choose a default profile in Settings > Browser.",
            vw_ask::agent_text(&outcome, "choosing a browser profile for this chat")
        ));
    }
    if crate::stops::generation(shared, session) != stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    picked.ok_or_else(unavailable)
}

/// The agent's result after the person's No on the profile card: the browser stays unused in this chat until the
/// user asks for it again, so the agent neither retries the open nor asks about profiles itself.
fn profile_declined(browser: &str) -> String {
    format!(
        "The user chose not to use {browser} in this chat now. Do not open it again or ask about browser profiles unless the user asks. No action ran, so report this step as not done."
    )
}

/// The chat's running turn, when Vibe reported one.
fn active_turn(shared: &Shared, session: &str) -> Option<String> {
    shared.store.view(session).and_then(|view| {
        view.state["session"]["status"]["activeTurnId"]
            .as_str()
            .filter(|turn| !turn.is_empty())
            .map(str::to_owned)
    })
}

fn declined_turn(calls: &Calls, session: &str) -> Option<String> {
    calls
        .declined
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(session)
        .cloned()
}

/// A browser profile card's answer: Yes must name one offered profile, which the held call then selects.
fn profile_answer(entry: &Value, output: &Value) -> Result<Option<String>, CoreError> {
    if entry["detail"]["effect"]["kind"] != "browserProfile"
        || output["decision"]["type"] != "approve"
    {
        return Ok(None);
    }
    let offered = output["profileId"].as_str().filter(|profile| {
        entry["detail"]["effect"]["choices"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["profileId"] == *profile))
    });
    let Some(profile) = offered else {
        return Err(CoreError::new(
            ErrorCode::CallbackRefused,
            "This answer is not offered.",
        ));
    };
    Ok(Some(profile.to_owned()))
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
        .ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE.to_owned())
        .and_then(|calls| {
            receipts::verify(
                calls,
                owner,
                entry["sessionId"].as_str().unwrap_or_default(),
                entry["id"].as_str().unwrap_or_default(),
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
    #[cfg(feature = "agent-vault")]
    if vault_save::is_card(shared, session, id) {
        return vault_save::answer(shared, origin, session, id, output, by);
    }
    let refused = || {
        CoreError::new(
            ErrorCode::CallbackRefused,
            "This permission is no longer open.",
        )
    };
    let Some(calls) = calls(shared) else {
        return Err(refused());
    };
    let Some(entry) = lock(calls)
        .iter()
        .find(|c| c.session == session && c.entry["id"] == id)
        .map(|c| c.entry.clone())
    else {
        return Err(refused());
    };
    if entry["detail"]["effect"]["kind"] == "browserProfile"
        && crate::command_origin::debug_api_acts(shared, origin)
    {
        // As for the selectProfile command: an account choice is the person's own, never the Debug API's.
        return Err(CoreError::new(
            ErrorCode::CallbackRefused,
            "Choosing this chat's browser profile requires the user. Ask the user to pick it on the card.",
        ));
    }
    let picked = profile_answer(&entry, output)?;
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
    // Kept only once this answer holds the card, so a refused second answer cannot replace the pick.
    if let Some(profile) = picked {
        if let Some(call) = lock(calls)
            .iter_mut()
            .find(|c| c.session == session && c.entry["id"] == id)
        {
            call.entry["detail"]["effect"]["picked"] = Value::String(profile.clone());
        }
        calls
            .picked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned(), profile);
    }
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
        if let Some(record) = shared.asks.outcome(&format!("callback:{id}"))
            && entry["state"]["status"] != "answered"
        {
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
            .ok_or(vw_vocabulary::texts::AGENT_PROMPT_LIMIT)?;
        records.remove(index);
    }
    Ok(())
}
#[cfg(test)]
#[path = "tool_calls_transport_tests.rs"]
mod transport_tests;

#[cfg(test)]
#[path = "tool_calls_cap_tests.rs"]
mod cap_tests;

#[cfg(test)]
#[path = "tool_calls_read_tests.rs"]
mod read_tests;

#[cfg(test)]
#[path = "tool_calls_suggestion_tests.rs"]
mod suggestion_tests;

/// Cancel only cards bound to the exact browser identity the person disabled or removed.
pub(crate) fn cancel_browser_profile(shared: &Shared, profile: &str) {
    let Some(calls) = calls(shared) else { return };
    let entries: Vec<_> = lock(calls)
        .iter()
        .filter(|call| call.entry["vwBrowserProfileId"] == profile)
        .filter_map(|call| call.entry["id"].as_str().map(str::to_owned))
        .collect();
    for id in entries {
        shared.asks.settle(&format!("callback:{id}"), Outcome::Cancelled {
            reason: "This browser profile was disconnected. Use a connected profile, or ask the user to reconnect it in Settings > Browser.".into(),
        });
        shared.ask_grants.forget_callback(&id);
    }
}

/// An open card still waits on this exact browser identity (a removed profile keeps its Settings row until then).
pub(crate) fn browser_profile_waiting(shared: &Shared, profile: &str) -> bool {
    let Some(calls) = calls(shared) else {
        return false;
    };
    lock(calls).iter().any(|call| {
        call.entry["vwBrowserProfileId"]
            .as_str()
            .is_some_and(|id| crate::chrome_control::same_profile(id, profile))
            && call.entry["id"]
                .as_str()
                .is_some_and(|id| shared.asks.get(&format!("callback:{id}")).is_some())
    })
}

/// Test-only: an open browser card bound to `profile`; dropping the returned handle settles it.
#[cfg(all(test, feature = "your-edge"))]
pub(crate) fn open_browser_card_for_test(
    shared: &Arc<Shared>,
    session: &str,
    profile: &str,
) -> Option<super::service::Handle> {
    let id = format!("{PREFIX}presence-test");
    let entry = json!({"id": id, "sessionId": session, "toolName": "mcp_vw.chrome_click", "vwBrowserProfileId": profile});
    lock(calls(shared)?).push_back(Call {
        session: session.into(),
        entry: entry.clone(),
    });
    let mut handle = super::callback_opened_at(shared, &entry, "approval", session, None)?;
    handle.cancel_on_drop = true;
    Some(handle)
}

/// A person changes this chat's profile; cancel only its browser cards, leaving sibling chats alone.
pub(crate) fn cancel_browser_chat(shared: &Shared, chat: &str) {
    let Some(calls) = calls(shared) else { return };
    let entries: Vec<_> = lock(calls)
        .iter()
        .filter(|call| call.session == chat && call.entry["vwBrowserProfileId"].is_string())
        .filter_map(|call| call.entry["id"].as_str().map(str::to_owned))
        .collect();
    for id in entries {
        shared.asks.settle(&format!("callback:{id}"), Outcome::Cancelled {
            reason: "The user changed this chat's browser profile. Open a fresh session in the selected profile.".into(),
        });
        shared.ask_grants.forget_callback(&id);
    }
}
