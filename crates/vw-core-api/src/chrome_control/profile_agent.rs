//! Agent-facing profile facts and unavailable-account diagnoses. Uses only app-owned bindings and
//! authenticated inventory; page data and readable names never select or change a route.
use super::{agent_browser::Browser, profile_bindings, profiles};
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::PoisonError;

pub(super) fn unavailable(shared: &Shared, id: &str) -> String {
    format!(
        "{} is unavailable; no action ran. Use another connected profile, or ask the user to reconnect this profile in Settings > Browser.",
        profiles::label(&shared.chrome_control, id)
    )
}

pub(super) fn check(shared: &Shared, chat: &str, tool: &str, args: &Value) -> Result<(), String> {
    let acts = tool == "chrome_execute" || tool == "chrome_session" && args["action"] == "open";
    if !acts
        || tool == "chrome_execute"
            && super::gateway::validate_session(shared, chat, tool, args).is_err()
        || super::output_cache::is_continuation(args)
    {
        return Ok(());
    }
    let requested = super::receipt::profile(shared, tool, args);
    let id = requested
        .as_deref()
        .filter(|id| {
            ![
                "chrome",
                "edge",
                "safari",
                "firefox",
                vw_vocabulary::texts::CHROME_NAME,
                vw_vocabulary::texts::EDGE_NAME,
            ]
            .contains(id)
        })
        .map(str::to_owned)
        .or_else(|| profile_bindings::selected(&shared.chrome_control, chat));
    let Some(id) = id else { return Ok(()) };
    profile_bindings::check(&shared.chrome_control, chat, &id)?;
    let browser = Browser::from_profile(&id);
    if !browser.enabled(shared) {
        return Ok(());
    }
    let rows = profiles::view(shared);
    if tool == "chrome_execute"
        && rows
            .iter()
            .any(|row| row.eligible && browser.owns_profile(&row.profile_id))
        && rows
            .iter()
            .any(|row| super::profile_ids::same(&row.profile_id, &id) && !row.eligible)
    {
        return Err(unavailable(shared, &id));
    }
    profiles::check(&shared.chrome_control, &id)
}

/// One inventory row as the agent sees it: the profile's agent name (with the account's domain), never the signed-in
/// account shown on screen.
fn agent_row(chrome: &super::ChromeControl, row: &super::BrowserProfile) -> Value {
    let name = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .agent_name(&row.profile_id);
    let mut row = json!(row);
    if let Some(row) = row.as_object_mut() {
        row.remove("account");
        row.insert("name".into(), json!(name));
    }
    row
}

/// Kept outside page content so selected identity and its recovery instruction survive page compaction.
pub(super) fn status(shared: &Shared, chat: &str, result: &mut Value) {
    let chrome = &shared.chrome_control;
    let choices = profiles::view(shared);
    let selected = profile_bindings::selected(chrome, chat);
    if choices.is_empty() && selected.is_none() {
        return;
    }
    let mut selected_row = selected.as_ref().map(|id| {
        choices.iter().find(|row| super::profile_ids::same(&row.profile_id, id)).map_or_else(|| {
            json!({"profileId":id,"name":chrome.profiles.lock().unwrap_or_else(PoisonError::into_inner).agent_name(id),"browser":Browser::from_profile(id).id(),"state":"unavailable","eligible":false,"reason":"Removed"})
        }, |row| agent_row(chrome, row))
    });
    if let (Some(id), Some(row)) = (selected.as_deref(), selected_row.as_mut()) {
        if let Err(reason) = profile_bindings::check(chrome, chat, id) {
            row["eligible"] = json!(false);
            row["instruction"] = json!(reason);
        } else if row["eligible"] != true {
            row["instruction"] = json!(unavailable(shared, id));
        }
        if let Some(selection) = super::profile_selection::view(chrome)
            .into_iter()
            .flatten()
            .find(|row| row.session_id == chat)
        {
            row["selectionState"] = json!(selection.state);
        }
        if let Some(text) = choice(shared, chat, id) {
            row["profileChoice"] = json!(text);
        }
    }
    let Some(body) = result
        .get_mut("structuredContent")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    body.insert("selectedProfile".into(), json!(selected_row));
    body.insert(
        "profileChoices".into(),
        Value::Array(choices.iter().map(|row| agent_row(chrome, row)).collect()),
    );
    if selected.is_none() {
        body.insert("profileSelection".into(), json!("A new chat uses the only connected profile or the user's default profile. With several profiles and no default, open with the browser's name as profileId (\"chrome\" or \"edge\"), not a profile's ID; the user picks on a card. The user can set a default in Settings > Browser."));
    }
    super::output::sync_text(result);
}

/// Who chose `profile` as this chat's profile: the person's pick on the profile card, or the person's default
/// profile. `None` when the chat is not bound to it or neither decided (the only connected profile, an older binding).
fn choice(shared: &Shared, chat: &str, profile: &str) -> Option<String> {
    let chrome = &shared.chrome_control;
    if !profile_bindings::selected(chrome, chat)
        .is_some_and(|bound| super::profile_ids::same(&bound, profile))
    {
        return None;
    }
    let name = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .agent_name(profile);
    let picked = chrome
        .picked_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(chat)
        .is_some_and(|picked| super::profile_ids::same(picked, profile));
    if picked {
        Some(format!("The user picked {name} for this chat on the card."))
    } else if profiles::view(shared)
        .iter()
        .any(|row| row.is_default && super::profile_ids::same(&row.profile_id, profile))
    {
        Some(format!(
            "This chat uses {name}, the user's default profile."
        ))
    } else {
        None
    }
}

/// An open result says who chose the chat's profile (`choice`), so the agent reports the person's own choice.
pub(super) fn chosen(shared: &Shared, chat: &str, tool: &str, args: &Value, result: &mut Value) {
    if tool != "chrome_session" || args["action"] != "open" {
        return;
    }
    let Some(text) = result["structuredContent"]["profileId"]
        .as_str()
        .and_then(|profile| choice(shared, chat, profile))
    else {
        return;
    };
    result["structuredContent"]["profileChoice"] = json!(text);
    super::output::sync_text(result);
}

/// Apply profile labels to the authenticated inventory, never to another profile's tab payload.
pub(super) fn names(chrome: &super::ChromeControl, result: &mut Value) {
    let Some(rows) = result["structuredContent"]["profiles"].as_array_mut() else {
        return;
    };
    let saved = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for row in rows {
        if let Some(id) = row["profileId"].as_str() {
            row["name"] = json!(saved.agent_name(id));
        }
    }
}

/// A complete authenticated inventory must contain the exact live profile before an open is sent.
pub(super) fn fresh(shared: &Shared, args: &Value, status: &Value) -> Result<(), String> {
    let (Some(id), Some(rows)) = (args["profileId"].as_str(), status["profiles"].as_array()) else {
        return Ok(());
    };
    if rows.iter().any(|row| {
        row["connected"] == true
            && row["profileId"]
                .as_str()
                .is_some_and(|other| super::profile_ids::same(id, other))
    }) {
        Ok(())
    } else {
        Err(unavailable(shared, id))
    }
}

/// Preserve the existing native connection diagnosis while naming the unavailable exact account.
pub(super) fn failed_connection(
    shared: &Shared,
    chat: &str,
    tool: &str,
    args: &Value,
    result: &mut Value,
) {
    if !(tool == "chrome_execute" || tool == "chrome_session" && args["action"] == "open")
        || result["structuredContent"]["error"]["code"] != "OWNER_REQUIRED"
        || result["structuredContent"]["status"] == "unknown"
    {
        return;
    }
    let id = profile_bindings::selected(&shared.chrome_control, chat)
        .or_else(|| super::receipt::profile(shared, tool, args));
    if let Some(id) = id
        && profiles::view(shared)
            .iter()
            .any(|row| super::profile_ids::same(&row.profile_id, &id) && !row.eligible)
    {
        result["structuredContent"]["profileAvailability"] = json!(unavailable(shared, &id));
        super::output::sync_text(result);
    }
}
