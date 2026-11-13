//! Person-owned chat profile handover. Fences old leases before awaiting their exact release receipts;
//! a durable pending change blocks actions and survives unknown results, retries and helper restarts.
use super::{profile_bindings, profiles};
use crate::core::Shared;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, sync::PoisonError, time::Duration};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Release {
    pub(super) id: String,
    pub(super) profile: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Change {
    pub(super) target: String,
    pub(super) releases: Vec<Release>,
    pub(super) state: String,
}

/// Selected account and an unfinished handover, shown in the chat picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChatBrowserProfile {
    /// Chat or New chat draft owner.
    pub session_id: String,
    /// Exact selected identity, or null when a legacy chat requires a choice.
    pub profile_id: Option<String>,
    /// Readable browser and person-owned profile name.
    pub name: String,
    /// Selected, stopping, unconfirmed, or removed (the browser deleted this profile).
    pub state: String,
    /// Requested identity, present only during a handover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub requested_profile_id: Option<String>,
}

pub(super) fn view(chrome: &super::ChromeControl) -> Option<Vec<ChatBrowserProfile>> {
    let chats = chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let pending = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pending
        .clone();
    let mut rows: Vec<_> = chats
        .into_iter()
        .map(|(session_id, profile_id)| {
            let change = pending.get(&session_id);
            let state = match change {
                Some(change) => change.state.clone(),
                // The browser deleted this profile; the chat names it and asks for another or a reconnect.
                None if profiles::deleted(chrome, &profile_id) => "removed".into(),
                None => "selected".into(),
            };
            ChatBrowserProfile {
                name: profiles::label(chrome, &profile_id),
                profile_id: Some(profile_id),
                state,
                requested_profile_id: change.map(|change| change.target.clone()),
                session_id,
            }
        })
        .collect();
    for chat in chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .ambiguous
        .keys()
    {
        if !rows.iter().any(|row| &row.session_id == chat) {
            rows.push(ChatBrowserProfile {
                session_id: chat.clone(),
                profile_id: None,
                name: "Choose profile".into(),
                state: "choiceRequired".into(),
                requested_profile_id: None,
            });
        }
    }
    rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    (!rows.is_empty()).then_some(rows)
}

/// Connected now; for a pick on the profile card also offline (paired and on, its extension asleep), since the open
/// that follows wakes the browser and still runs only in a live profile.
fn eligible(shared: &Shared, profile: &str, offline: bool) -> Result<(), String> {
    if profiles::view(shared).iter().any(|row| {
        (row.eligible || offline && row.state == super::ProfileState::Offline)
            && super::profile_ids::same(&row.profile_id, profile)
    }) {
        Ok(())
    } else {
        Err(format!(
            "{} is unavailable; no action ran. Use another connected profile, or ask the user to reconnect this profile in Settings > Browser.",
            profiles::label(&shared.chrome_control, profile)
        ))
    }
}

/// Returns whether a previous task was released, so the person command can open a fresh task afterwards.
pub(super) async fn select(shared: &Shared, chat: &str, profile: &str) -> Result<bool, String> {
    select_with(shared, chat, profile, false).await
}

/// The person's pick on the profile card (`ask::tool_calls::choose_profile`): as [`select`], and an offline profile
/// the card offered may be picked.
pub(super) async fn select_picked(
    shared: &Shared,
    chat: &str,
    profile: &str,
) -> Result<bool, String> {
    let released = select_with(shared, chat, profile, true).await?;
    shared
        .chrome_control
        .picked_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(chat.into(), profiles::canonical(profile));
    Ok(released)
}

async fn select_with(
    shared: &Shared,
    chat: &str,
    profile: &str,
    offline: bool,
) -> Result<bool, String> {
    if !crate::new_chat::known_owner(shared, chat) {
        return Err(
            "This chat is no longer open. Open the chat and select its profile again.".into(),
        );
    }
    let chrome = &shared.chrome_control;
    let _held = chrome.binding_changes.lock().await;
    eligible(shared, profile, offline)?;
    let profile = profiles::canonical(profile);
    let pending = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pending
        .get(chat)
        .cloned();
    if pending.is_none()
        && profile_bindings::selected(chrome, chat)
            .as_deref()
            .is_some_and(|old| super::profile_ids::same(old, &profile))
        && profile_bindings::check(chrome, chat, &profile).is_ok()
    {
        return Ok(false);
    }
    let fresh = pending.is_none();
    let mut change = pending.unwrap_or_else(|| Change {
        target: profile.clone(),
        releases: releases(chrome, chat),
        state: "stopping".into(),
    });
    if change.target != profile {
        return Err("The previous profile switch is still unconfirmed. Retry that selection after reconnecting the old profile.".into());
    }
    profile_bindings::begin_change(chrome, chat, change.clone())?;
    crate::ask::tool_calls::cancel_browser_chat(shared, chat);
    {
        let mut tasks = chrome.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        let mut work = chrome.work.lock().unwrap_or_else(PoisonError::into_inner);
        for release in &change.releases {
            tasks.remove(&release.id);
            work.ask_stop(&release.id, chat);
        }
    }
    super::owner::publish(shared);
    for release in &change.releases {
        if !release_acknowledged(chrome, chat, release, fresh).await {
            change.state = "unconfirmed".into();
            profile_bindings::begin_change(chrome, chat, change)?;
            super::owner::publish(shared);
            return Err("The old profile's release is unconfirmed; switching is stopped. Reconnect the old profile and retry this selection.".into());
        }
    }
    eligible(shared, &profile, offline)?;
    profile_bindings::complete_change(chrome, chat, &profile)?;
    super::owner::publish(shared);
    Ok(!change.releases.is_empty())
}

fn releases(chrome: &super::ChromeControl, chat: &str) -> Vec<Release> {
    let saved = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .leases
        .get(chat)
        .cloned()
        .unwrap_or_default();
    let work = chrome
        .work
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .releases(chat);
    saved
        .into_iter()
        .chain(work)
        .map(|release| (release.id.clone(), release))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect()
}

async fn release_acknowledged(
    chrome: &super::ChromeControl,
    chat: &str,
    release: &Release,
    fresh: bool,
) -> bool {
    let same_owner = || {
        chrome
            .bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .check_session(chat, &release.id, &release.profile)
            .is_ok()
    };
    // Positive browser deletion proves that this profile's runtime and extension are gone.
    // Its locally fenced lease cannot acknowledge Stop; an offline profile without that proof still must.
    if same_owner() && profiles::deleted(chrome, &release.profile) {
        return true;
    }
    for attempt in 0..6 {
        if !same_owner() {
            return false;
        }
        let action = if fresh && attempt == 0 { "stop" } else { "get" };
        let answer = chrome
            .owner
            .call(
                "chrome_session",
                json!({"action":action,"sessionId":release.id}),
            )
            .await;
        let body = &answer.value()["structuredContent"];
        if !same_owner()
            || body["sessionId"] != release.id
            || !body["profileId"]
                .as_str()
                .is_some_and(|id| super::profile_ids::same(id, &release.profile))
        {
            return false;
        }
        if body["state"] == "stopped"
            && body["capture"] == "off"
            && body["status"] != "unknown"
            && !body["error"].is_object()
        {
            return true;
        }
        if body["state"] != "stopping" {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

pub(super) async fn command(
    core: &crate::Core,
    origin: crate::Origin,
    chat: &str,
    profile: &str,
) -> Result<crate::CommandResult, crate::CoreError> {
    if crate::command_origin::debug_api_acts(&core.shared, origin) {
        return Err(super::commands::invalid(
            "Choosing this chat's browser profile requires the user. Open a browser session so the user can pick a profile on the card.",
        ));
    }
    let opened = select(&core.shared, chat, profile)
        .await
        .map_err(|reason| super::commands::invalid(&reason))?;
    if opened && core.shared.store.view(chat).is_some() {
        let answer = super::gateway::call(
            &core.shared,
            chat,
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .map_err(|reason| super::commands::invalid(&reason))?;
        if answer["vwMcpResult"]["isError"] == true
            || answer["vwMcpResult"]["structuredContent"]["status"] == "unknown"
        {
            return Err(super::commands::invalid(
                "The new profile is selected, but its task could not open. Reconnect this profile and open a new task.",
            ));
        }
    }
    Ok(crate::CommandResult::ChromeControl {
        view: super::commands::view(&core.shared),
        offer: None,
        hint: None,
    })
}
