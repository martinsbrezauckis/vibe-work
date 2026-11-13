//! Resolves a new chat's browser choice from authenticated status and person-owned defaults.
//! Called before open dispatch; exact model arguments cannot choose among several unselected accounts: with several
//! and no default the person picks on a card (`choice_needed`, `ask::tool_calls::choose_profile`). An explicit
//! default is never replaced silently: while it is not connected the card asks instead.
use super::{agent_browser::Browser, profile_ids, profiles};
use crate::core::Shared;
use serde_json::Value;
use std::sync::PoisonError;

/// The profiles the person must choose from before this open: an unbound chat whose browser has a default that is
/// not connected now, or no default and more than one paired profile (connected or not); an older chat whose tasks
/// used several profiles, or a chat whose profile the browser deleted or a newer identity of the same profile
/// replaced. Only connected profiles are offered. `None` when the connected default, the only paired profile or the
/// chat's own binding decides (or the request names no profile, or nothing is connected to offer).
pub(super) fn choice_needed(
    shared: &Shared,
    chat: &str,
    args: &Value,
) -> Option<Vec<crate::ask::tool_calls::ProfileChoice>> {
    let requested = args["profileId"].as_str()?;
    let chrome = &shared.chrome_control;
    let bound = super::profile_bindings::selected(chrome, chat);
    let ambiguous = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .ambiguous
        .contains_key(chat);
    let browser = Browser::from_profile(bound.as_deref().unwrap_or(requested));
    let paired: Vec<_> = profiles::view(shared)
        .into_iter()
        .filter(|row| {
            row.state != super::ProfileState::Unavailable && browser.owns_profile(&row.profile_id)
        })
        .collect();
    let default_closed = paired
        .iter()
        .any(|row| row.is_default && row.state != super::ProfileState::Connected);
    let several = paired.len() > 1;
    let choices: Vec<_> = paired
        .into_iter()
        .filter(|row| row.state == super::ProfileState::Connected)
        .collect();
    let ask = match &bound {
        Some(profile) => {
            (profiles::deleted(chrome, profile) || profiles::retired(chrome, profile))
                && !choices.is_empty()
        }
        None if ambiguous => !choices.is_empty(),
        None => {
            !choices.is_empty()
                && (default_closed || (several && !choices.iter().any(|row| row.is_default)))
        }
    };
    tracing::debug!(
        bound = bound.is_some(),
        deleted = bound
            .as_deref()
            .is_some_and(|id| profiles::deleted(chrome, id)),
        ambiguous,
        choices = choices.len(),
        ask,
        "browser profile choice"
    );
    ask.then(|| {
        choices
            .into_iter()
            .map(|row| crate::ask::tool_calls::ProfileChoice {
                agent_name: chrome
                    .profiles
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .agent_name(&row.profile_id),
                profile_id: row.profile_id,
                name: row.name,
                account: row.account,
            })
            .collect()
    })
}

/// Before an open takes the binding lock: when the person must choose, hold the open on the profile card, select
/// the picked profile as the person's own selection and open in it. Waiting here never blocks other chats' opens.
pub(super) async fn ask_person(
    shared: &std::sync::Arc<Shared>,
    chat: &str,
    args: &mut Value,
) -> Result<(), String> {
    let Some(choices) = choice_needed(shared, chat, args) else {
        return Ok(());
    };
    let browser = choices.first().map_or(Browser::Chrome, |row| {
        Browser::from_profile(&row.profile_id)
    });
    let closed = closed_windows(shared, browser);
    let picked = crate::ask::tool_calls::choose_profile(
        shared,
        chat,
        browser.short_name(),
        &choices,
        &closed,
    )
    .await?;
    super::profile_selection::select_picked(shared, chat, &picked).await?;
    args["profileId"] = Value::String(picked);
    Ok(())
}

/// Which paired profiles of `browser` the card cannot offer because they are not connected now, for its one-line
/// note: the default's shown name when the default is one of them (`closedDefault`), else whether any other paired
/// profile is (`closedOthers`).
fn closed_windows(shared: &Shared, browser: Browser) -> Value {
    let paired: Vec<_> = profiles::view(shared)
        .into_iter()
        .filter(|row| {
            row.state != super::ProfileState::Unavailable && browser.owns_profile(&row.profile_id)
        })
        .collect();
    let closed = |row: &&profiles::BrowserProfile| row.state != super::ProfileState::Connected;
    match paired.iter().filter(closed).find(|row| row.is_default) {
        Some(default) => serde_json::json!({"closedDefault": default.name}),
        None => serde_json::json!({"closedOthers": paired.iter().any(|row| closed(&row))}),
    }
}

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
            return Err("This chat uses another browser profile; no action ran. Continue in this chat's profile, or ask the user to start a new chat for the other profile.".into());
        }
        bound
    } else {
        let mine: Vec<_> = profiles::view(shared)
            .into_iter()
            .filter(|row| browser.owns_profile(&row.profile_id))
            .collect();
        let closed_default = mine.iter().find(|row| {
            row.is_default
                && matches!(
                    row.state,
                    super::ProfileState::Offline | super::ProfileState::ExtensionMissing
                )
        });
        if let Some(default) = closed_default.filter(|_| mine.iter().any(|row| row.eligible)) {
            // The person chose this default; another connected profile never stands in for it. With nothing
            // connected the open goes on below and the browser start brings its profiles back.
            return Err(format!(
                "{} is the default for new chats, and it is not connected now; no action ran. Ask the user to open that profile's window, or to pick a profile when Vibe Work asks.",
                profiles::label(chrome, &default.profile_id)
            ));
        }
        let choices: Vec<_> = mine.into_iter().filter(|row| row.eligible).collect();
        let choice = choices
            .iter()
            .find(|row| row.is_default)
            .or_else(|| (choices.len() == 1).then(|| &choices[0]));
        if let Some(choice) = choice {
            if !shorthand && !profile_ids::same(&choice.profile_id, &requested) {
                return Err(format!(
                    "{} is the default for new chats; no action ran. Use its profile ID, or ask the user to change the default profile in Settings > Browser.",
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
                "Several {} profiles are available: {names}. No action ran. Open the session again so the user can pick one, or ask the user to choose a default profile in Settings > Browser.",
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
