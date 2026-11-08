//! Value-free save cards for the authenticated vault capture broker.
//! Reuses the app-owned callback overlay and ask lifecycle; only the person's window may decide.
use super::{Arc, Shared, Value, json};
use crate::{CommandResult, CoreError, CoreEvent, ErrorCode, Origin};
use vw_ask::{AnsweredBy, Choice, Outcome, Scope};
use vw_keys::CaptureChoice;

pub(crate) const KIND: &str = "vault_save";

pub(crate) async fn ask(
    shared: &Arc<Shared>,
    session: &str,
    name: &str,
    source: &str,
    duplicate: bool,
    replace: bool,
    tool: &str,
) -> Result<CaptureChoice, String> {
    let id = format!(
        "{}{}",
        super::PREFIX,
        vw_agent_tools::credential::new_credential()
            .map_err(|_| "The save card could not open; try again.")?
    );
    let choices = if duplicate {
        if replace {
            json!(["approve", "keep_both", "deny"])
        } else {
            json!(["keep_both", "deny"])
        }
    } else {
        json!(["approve", "deny"])
    };
    let entry = json!({"id":id,"sessionId":session,"callbackId":id,"type":"callback",
        "createdAt":shared.asks.now_ms(),"vwToolAsk":true,"vwVaultSave":{"duplicate":duplicate,"replace":replace},
        "detail":{"kind":"approval","choices":choices,"requiredPermissions":[],
            "effect":{"kind":"tool","toolName":format!("mcp_vw.{tool}"),"input":{},
                "display":{"summary":format!("{} · {}", vw_gate::dialog_quote(name), source)}}},
        "state":{"status":"open"}});
    let stop = crate::stops::generation(shared, session);
    {
        let mut records = super::lock(
            super::calls(shared)
                .ok_or("The tool service is unavailable; reopen the chat and try again.")?,
        );
        super::make_room(shared, &mut records)?;
        records.push_back(super::Call {
            session: session.into(),
            entry,
        });
    }
    let mut handle = shared.asks.open_own(
        KIND,
        Some(session.into()),
        vw_ask::Requester::Agent,
        vec![Choice::Once, Choice::No],
        Some(format!("callback:{id}")),
    );
    handle.cancel_on_drop = true;
    crate::stops::opened_after_stop(shared, session, stop, &handle.ask.id);
    shared.emit(CoreEvent::CallbackOpened {
        session_id: session.into(),
        callback_id: id.clone(),
        kind: "approval".into(),
        subagent: None,
    });
    super::changed(shared, session);
    let outcome = handle.closed().await;
    super::changed(shared, session);
    if !matches!(outcome, Outcome::Allowed { .. }) {
        return Err(vw_ask::agent_text(&outcome, "this vault save"));
    }
    if crate::stops::generation(shared, session) != stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    let records = super::lock(
        super::calls(shared)
            .ok_or("The tool service is unavailable; reopen the chat and try again.")?,
    );
    let answer = records
        .iter()
        .find(|c| c.entry["id"] == id)
        .and_then(|c| c.entry["state"]["output"]["decision"]["type"].as_str());
    match answer {
        Some("keep_both") if duplicate => Ok(CaptureChoice::KeepBoth),
        Some("approve") if duplicate && replace => Ok(CaptureChoice::Replace),
        Some("approve") if !duplicate => Ok(CaptureChoice::New),
        _ => Err("The save choice is unavailable; ask the user to try again.".into()),
    }
}

pub(super) fn is_card(shared: &Shared, session: &str, id: &str) -> bool {
    super::calls(shared).is_some_and(|calls| {
        super::lock(calls).iter().any(|c| {
            c.session == session && c.entry["id"] == id && c.entry["vwVaultSave"].is_object()
        })
    })
}

pub(super) fn answer(
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
            "This save choice is no longer offered; request the save again.",
        )
    };
    if origin != Origin::Ui {
        let mut error = CoreError::new(
            ErrorCode::Denied,
            "The vault save needs the person's answer; answer it in Vibe Work's window.",
        );
        error.server_code = Some(crate::vault_scope::VAULT_USER_ONLY.into());
        return Err(error);
    }
    let decision = output["decision"]["type"].as_str().ok_or_else(refused)?;
    let calls = super::calls(shared).ok_or_else(refused)?;
    let mut records = super::lock(calls);
    let record = records
        .iter_mut()
        .find(|c| c.session == session && c.entry["id"] == id)
        .ok_or_else(refused)?;
    if output["type"] != "approval"
        || !record.entry["detail"]["choices"]
            .as_array()
            .is_some_and(|cs| cs.iter().any(|c| c == decision))
    {
        return Err(refused());
    }
    let delivery = shared
        .asks
        .reserve(&format!("callback:{id}"), by)
        .ok_or_else(refused)?;
    // Retain only the offered word; arbitrary client fields never enter history or receipts.
    record.entry["state"] =
        json!({"status":"answered","output":{"type":"approval","decision":{"type":decision}}});
    drop(records);
    shared.emit(CoreEvent::CallbackAnswered {
        session_id: session.into(),
        callback_id: id.into(),
        origin,
    });
    delivery.confirmed(if decision == "deny" {
        crate::ask::refused(by, None)
    } else {
        crate::ask::allowed(by, Scope::Once)
    });
    super::changed(shared, session);
    Ok(CommandResult::Reply {
        result: Value::Null,
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
    })
}
