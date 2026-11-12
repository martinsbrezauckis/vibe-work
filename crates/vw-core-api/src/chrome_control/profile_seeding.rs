//! Resolves a new chat's browser choice from authenticated status and person-owned defaults.
//! Called before open dispatch; exact model arguments cannot choose among several unselected accounts.
use super::{agent_browser::Browser, profile_ids, profiles};
use crate::core::Shared;
use serde_json::Value;
use std::sync::PoisonError;

pub(super) fn resolve(
    shared: &Shared,
    chat: &str,
    args: &mut Value,
    status: &Value,
) -> Result<(), String> {
    let Some(requested) = args["profileId"].as_str().map(str::to_owned) else {
        return Ok(());
    };
    let chrome = &shared.chrome_control;
    profiles::observe(chrome, status);
    super::profile_bindings::migrate(chrome, chat, status)?;
    let browser = Browser::from_profile(&requested);
    let shorthand = [browser.id(), browser.name(), browser.short_name()]
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&requested));
    let bound = chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(chat)
        .cloned();
    let selected = if let Some(bound) = bound {
        super::profile_bindings::check(chrome, chat, &bound)?;
        if !shorthand && !profile_ids::same(&bound, &requested) {
            return super::profile_bindings::check(chrome, chat, &requested);
        }
        if shorthand && !browser.owns_profile(&bound) {
            return Err("This chat is bound to another browser profile; no action ran. Ask the user to select the intended profile in this chat.".into());
        }
        bound
    } else {
        let choices: Vec<_> = profiles::view(shared)
            .into_iter()
            .filter(|row| row.eligible && browser.owns_profile(&row.profile_id))
            .collect();
        let choice = choices
            .iter()
            .find(|row| row.is_default)
            .or_else(|| (choices.len() == 1).then(|| &choices[0]));
        if let Some(choice) = choice {
            if !shorthand && !profile_ids::same(&choice.profile_id, &requested) {
                return Err(format!(
                    "{} is selected for new chats; no action ran. Use its profile ID or ask the user to select another profile in this chat.",
                    profiles::label(chrome, &choice.profile_id)
                ));
            }
            choice.profile_id.clone()
        } else if choices.len() > 1 {
            let names = choices
                .iter()
                .map(|row| {
                    format!(
                        "{} [{}]",
                        profiles::label(chrome, &row.profile_id),
                        row.profile_id
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Several {} profiles are available: {names}. No action ran. Ask the user to select a profile in this chat.",
                browser.short_name()
            ));
        } else {
            // Older helpers may omit their inventory. Keep exact legacy bindings; never guess a shorthand.
            profile_ids::resolve_browser(args, status)?;
            return Ok(());
        }
    };
    let wire = status["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["profileId"].as_str())
        .find(|id| profile_ids::same(id, &selected))
        .unwrap_or(&selected);
    args["profileId"] = Value::String(wire.to_owned());
    Ok(())
}
