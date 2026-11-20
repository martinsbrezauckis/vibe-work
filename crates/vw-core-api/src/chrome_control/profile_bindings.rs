//! Durable browser bindings owned by the app. The gateway saves exact authenticated identities, never labels.
//! Helper restarts keep chat choices; owner changes and reused session IDs cannot retarget saved work.
use super::ChromeControl;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::Path};
use vw_platform::sync::LockRecover;
#[path = "profile_bindings_status.rs"]
mod status;
pub(super) use status::{migrate, observe, reconcile_completed_status, session_profile};

#[derive(Clone, Serialize, Deserialize)]
struct Identity {
    #[serde(default)]
    chat: Option<String>,
    profile: String,
    owner: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct UncertainOpen {
    id: Option<String>,
    profile: Option<String>,
    owner: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Bindings {
    #[serde(skip)]
    storage_unavailable: bool,
    chats: HashMap<String, Identity>,
    sessions: HashMap<String, Identity>,
    #[serde(default)]
    pub(super) pending: HashMap<String, super::profile_selection::Change>,
    #[serde(default)]
    pub(super) leases: HashMap<String, Vec<super::profile_selection::Release>>,
    #[serde(default)]
    pub(super) ambiguous: HashMap<String, Vec<String>>,
    #[serde(default)]
    uncertain_opens: HashMap<String, UncertainOpen>,
    #[serde(skip)]
    generations: HashMap<String, u64>,
    #[serde(skip)]
    owner: Option<String>,
}
impl Bindings {
    pub(super) fn load(dir: Option<&Path>) -> Self {
        let mut saved: Self =
            super::profile_storage::load(dir, "chat-profiles.json", 4 * 1024 * 1024)
                .unwrap_or_else(|()| Self {
                    storage_unavailable: true,
                    ..Self::default()
                });
        for (chat, leases) in &saved.leases {
            for lease in leases {
                if let Some(identity) = saved.sessions.get_mut(&lease.id) {
                    if identity.chat.as_ref().is_some_and(|old| old != chat) {
                        saved.storage_unavailable = true;
                    } else {
                        identity.chat = Some(chat.clone());
                    }
                }
            }
        }
        saved
    }
    pub(super) fn chats(&self) -> HashMap<String, String> {
        self.chats
            .iter()
            .map(|(chat, identity)| (chat.clone(), identity.profile.clone()))
            .collect()
    }
    pub(super) fn save(&self, dir: Option<&Path>) -> Result<(), String> {
        if self.storage_unavailable {
            return Err(super::profile_storage::UNREADABLE.into());
        }
        let Some(dir) = dir else { return Ok(()) };
        let failed = || {
            "The browser choice could not be saved. Check this profile before selecting it again."
                .to_owned()
        };
        std::fs::create_dir_all(dir).map_err(|_| failed())?;
        let bytes = serde_json::to_vec(self).map_err(|_| failed())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(failed());
        }
        vw_platform::replace_private(&dir.join("chat-profiles.json"), &bytes).map_err(|_| failed())
    }
    pub(super) fn observe_owner(&mut self, status: &Value) {
        if let Some(owner) = status["ownerPublicKey"]
            .as_str()
            .filter(|key| !key.is_empty())
        {
            self.owner = Some(owner.to_owned());
        }
    }
    fn check_owner(&self, identity: &Identity) -> Result<(), String> {
        if identity
            .owner
            .as_ref()
            .zip(self.owner.as_ref())
            .is_some_and(|(old, current)| old != current)
        {
            return Err("This profile was paired with another Vibe Work identity; no action ran. Ask the user to reconnect the profile in Settings > Browser, then open the browser session again.".into());
        }
        Ok(())
    }
    pub(super) fn check(&self, chat: &str, profile: &str) -> Result<(), String> {
        if self.storage_unavailable {
            return Err(super::profile_storage::UNREADABLE.into());
        }
        if self.uncertain_opens.contains_key(chat) {
            return Err(vw_vocabulary::texts::BROWSER_PROFILE_OPEN_UNCONFIRMED.into());
        }
        if self.ambiguous.contains_key(chat) {
            return Err("This older chat used several browser profiles; no action ran. Open the browser session again so the user can pick one.".into());
        }
        if self.pending.contains_key(chat) {
            return Err("This chat's profile switch is unconfirmed; no action ran. Reconnect the old profile and retry the selection.".into());
        }
        if let Some(identity) = self.chats.get(chat) {
            self.check_owner(identity)?;
            if !super::profile_ids::same(&identity.profile, profile)
                && self
                    .leases
                    .get(chat)
                    .is_some_and(|leases| !leases.is_empty())
            {
                return Err(OTHER_PROFILE.into());
            }
        }
        Ok(())
    }
    pub(super) fn check_session(
        &self,
        chat: &str,
        session: &str,
        profile: &str,
    ) -> Result<(), String> {
        if self.storage_unavailable {
            return Err(super::profile_storage::UNREADABLE.into());
        }
        if let Some(identity) = self.sessions.get(session) {
            self.check_owner(identity)?;
            if identity.chat.as_deref() != Some(chat) {
                return Err("This browser session could not be confirmed as belonging to this chat. Reconnect this profile before opening a new browser session.".into());
            }
            if !super::profile_ids::same(&identity.profile, profile) {
                return Err("This browser session is already associated with another profile. Reconnect this profile before opening a new browser session.".into());
            }
        }
        Ok(())
    }
}

/// Internal marker for a cross-profile binding refusal; render it after releasing the saved-binding lock.
const OTHER_PROFILE: &str = "OTHER_PROFILE";

fn other_profile(chrome: &ChromeControl, chat: &str) -> String {
    let selected = selected(chrome, chat);
    let fallback = {
        let saved = chrome.bindings.lock_or_recover();
        saved
            .chats
            .get(chat)
            .map(|identity| identity.profile.clone())
    };
    let bound = selected.or(fallback);
    let name = bound.map_or_else(
        || vw_vocabulary::texts::BROWSER_PROFILE_CURRENT.to_owned(),
        |id| super::profiles::label(chrome, &id),
    );
    vw_vocabulary::texts::BROWSER_PROFILE_AGENT_REFUSAL.replace("{profile}", &name)
}

/// A saved pick is a preference until an external session has been opened or still needs release.
pub(super) fn locked(chrome: &ChromeControl, chat: &str) -> bool {
    let saved = chrome.bindings.lock_or_recover();
    let saved = saved.uncertain_opens.contains_key(chat)
        || saved
            .leases
            .get(chat)
            .is_some_and(|leases| !leases.is_empty());
    saved || !chrome.work.lock_or_recover().releases(chat).is_empty()
}

pub(super) fn uncertain(chrome: &ChromeControl, chat: &str) -> bool {
    chrome
        .bindings
        .lock_or_recover()
        .uncertain_opens
        .contains_key(chat)
}

pub(super) fn uncertain_chats(chrome: &ChromeControl) -> Vec<String> {
    chrome
        .bindings
        .lock_or_recover()
        .uncertain_opens
        .keys()
        .cloned()
        .collect()
}

/// A sent open with an unconfirmed result can have created a live helper session.
pub(super) fn mark_uncertain(
    chrome: &ChromeControl,
    chat: &str,
    id: Option<&str>,
    profile: Option<&str>,
) {
    let mut saved = chrome.bindings.lock_or_recover();
    let owner = saved.owner.clone();
    saved.uncertain_opens.insert(
        chat.into(),
        UncertainOpen {
            id: id.filter(|id| !id.is_empty()).map(str::to_owned),
            profile: profile
                .filter(|profile| !profile.is_empty())
                .map(str::to_owned),
            owner,
        },
    );
    if let Err(reason) = saved.save(chrome.dir()) {
        tracing::warn!(%reason, "Uncertain browser open could not be saved");
    }
}

/// [`check`] for the agent's own calls: when the chat's profile is the one the person picked on the profile card, the
/// refusal of another profile names the picked one, so the agent continues there instead of guessing why it was
/// refused. Person commands keep [`check`]'s text.
pub(super) fn check_agent(chrome: &ChromeControl, chat: &str, profile: &str) -> Result<(), String> {
    let other = other_profile(chrome, chat);
    check(chrome, chat, profile).map_err(|refusal| {
        if refusal != other {
            return refusal;
        }
        let picked = chrome.picked_profiles.lock_or_recover().get(chat).cloned();
        match picked {
            Some(picked)
                if selected(chrome, chat)
                    .is_some_and(|bound| super::profile_ids::same(&bound, &picked)) =>
            {
                let name = chrome.profiles.lock_or_recover().agent_name(&picked);
                format!(
                    "{refusal} The user picked {name} for this chat; use it (profileId {picked})."
                )
            }
            _ => refusal,
        }
    })
}

pub(super) fn check(chrome: &ChromeControl, chat: &str, profile: &str) -> Result<(), String> {
    // Existing single-binding chats retain their exact choice, including legacy raw Chrome UUIDs.
    let bound = selected(chrome, chat);
    if let Some(bound) = bound.as_deref()
        && !super::profile_ids::same(bound, profile)
        && locked(chrome, chat)
    {
        return Err(other_profile(chrome, chat));
    }
    let result = chrome.bindings.lock_or_recover().check(chat, profile);
    result.map_err(|reason| {
        if reason == OTHER_PROFILE {
            other_profile(chrome, chat)
        } else {
            reason
        }
    })
}

pub(super) fn remember(
    chrome: &ChromeControl,
    chat: &str,
    profile: &str,
    session: Option<&str>,
) -> Result<(), String> {
    check(chrome, chat, profile)?;
    let mut saved = chrome.bindings.lock_or_recover();
    if let Some(session) = session {
        saved.check_session(chat, session, profile)?;
    }
    let identity = Identity {
        chat: Some(chat.into()),
        profile: profile.into(),
        owner: saved.owner.clone(),
    };
    if let Some(id) = session {
        let leases = saved.leases.entry(chat.into()).or_default();
        if !leases.iter().any(|lease| lease.id == id) {
            leases.push(super::profile_selection::Release {
                id: id.into(),
                profile: profile.into(),
            });
        }
        saved.sessions.insert(id.into(), identity.clone());
    }
    saved.chats.insert(chat.into(), identity);
    chrome
        .chat_profiles
        .lock_or_recover()
        .insert(chat.into(), profile.into());
    if let Err(reason) = saved.save(chrome.dir()) {
        tracing::warn!(%reason, "Browser binding could not be saved after open");
    }
    Ok(())
}

pub(super) fn forget(chrome: &ChromeControl, chat: &str) {
    let mut saved = chrome.bindings.lock_or_recover();
    saved.chats.remove(chat);
    saved.pending.remove(chat);
    saved.ambiguous.remove(chat);
    saved.leases.remove(chat);
    saved.uncertain_opens.remove(chat);
    if let Err(reason) = saved.save(chrome.dir()) {
        tracing::warn!(%reason, "Browser binding removal could not be saved");
    }
}

/// Current selection, including a legacy single binding that has not yet opened a new task.
pub(super) fn selected(chrome: &ChromeControl, chat: &str) -> Option<String> {
    chrome.chat_profiles.lock_or_recover().get(chat).cloned()
}
/// The refusal of a tab the Debug API picks outside this chat's profile: the person picks a chat's profile, so a chat
/// without one yet is told so instead of being told it uses another.
pub(super) fn tab_refusal(chrome: &ChromeControl, chat: &str) -> &'static str {
    if selected(chrome, chat).is_some() {
        "This chat uses another browser profile; nothing was added. Use a tab from this chat's profile, or ask the user to start a new chat for the other profile."
    } else {
        "This chat has no browser profile yet, and only the user picks it; nothing was added. Ask the user to add the tab from the composer, or open a browser session so the user can pick a profile."
    }
}
pub(super) fn generation(chrome: &ChromeControl, chat: &str) -> u64 {
    chrome
        .bindings
        .lock_or_recover()
        .generations
        .get(chat)
        .copied()
        .unwrap_or_default()
}
pub(super) fn begin_change(
    chrome: &ChromeControl,
    chat: &str,
    change: super::profile_selection::Change,
) -> Result<(), String> {
    let mut saved = chrome.bindings.lock_or_recover();
    if saved.uncertain_opens.contains_key(chat) {
        return Err(vw_vocabulary::texts::BROWSER_PROFILE_OPEN_UNCONFIRMED.into());
    }
    let mut next = saved.clone();
    if !next.pending.contains_key(chat) {
        *next.generations.entry(chat.into()).or_default() += 1;
    }
    next.pending.insert(chat.into(), change);
    next.save(chrome.dir())?;
    *saved = next;
    Ok(())
}
pub(super) fn complete_change(
    chrome: &ChromeControl,
    chat: &str,
    profile: &str,
) -> Result<(), String> {
    let mut saved = chrome.bindings.lock_or_recover();
    let mut next = saved.clone();
    next.chats.insert(
        chat.into(),
        Identity {
            chat: Some(chat.into()),
            profile: profile.into(),
            owner: next.owner.clone(),
        },
    );
    next.pending.remove(chat);
    next.ambiguous.remove(chat);
    next.leases.remove(chat);
    next.uncertain_opens.remove(chat);
    next.save(chrome.dir())?;
    *saved = next;
    chrome
        .chat_profiles
        .lock_or_recover()
        .insert(chat.into(), profile.into());
    Ok(())
}
/// Draft adoption copies the exact selection before the first turn, without inventing a new default.
pub(super) fn adopt(chrome: &ChromeControl, old: &str, new: &str) -> Result<(), String> {
    let Some(profile) = selected(chrome, old) else {
        return Ok(());
    };
    check(chrome, old, &profile)?;
    remember(chrome, new, &profile, None)?;
    forget(chrome, old);
    chrome.chat_profiles.lock_or_recover().remove(old);
    Ok(())
}
