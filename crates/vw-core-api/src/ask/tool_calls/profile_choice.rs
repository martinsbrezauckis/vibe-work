//! Browser profile cards: the agent's choice among paired profiles, the answer read from a card, and
//! cancelling or checking the cards bound to one profile or one chat.
use crate::{CoreError, CoreEvent, ErrorCode, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_ask::Outcome;
use vw_platform::sync::LockRecover;

use super::{Call, Calls, PREFIX, calls, changed, lock, make_room};

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
    let mut handle =
        crate::ask::callback_opened_at(shared, &entry, "approval", session, Some(stop))
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
    let picked = calls.picked.lock_or_recover().remove(&id);
    if matches!(outcome, Outcome::Refused { .. }) {
        if let Some(turn) = turn {
            calls
                .declined
                .lock_or_recover()
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
    calls.declined.lock_or_recover().get(session).cloned()
}

/// A browser profile card's answer: Yes must name one offered profile, which the held call then selects.
pub(super) fn profile_answer(entry: &Value, output: &Value) -> Result<Option<String>, CoreError> {
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
) -> Option<crate::ask::service::Handle> {
    let id = format!("{PREFIX}presence-test");
    let entry = json!({"id": id, "sessionId": session, "toolName": "mcp_vw.chrome_click", "vwBrowserProfileId": profile});
    lock(calls(shared)?).push_back(Call {
        session: session.into(),
        entry: entry.clone(),
    });
    let mut handle = crate::ask::callback_opened_at(shared, &entry, "approval", session, None)?;
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
