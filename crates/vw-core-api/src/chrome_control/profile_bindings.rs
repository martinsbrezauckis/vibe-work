//! Durable browser bindings owned by the app. The gateway saves exact authenticated identities, never labels.
//! Helper restarts keep chat choices; owner changes and reused session IDs cannot retarget saved work.
use super::ChromeControl;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::Path, sync::PoisonError};

#[derive(Clone, Serialize, Deserialize)]
struct Identity {
    #[serde(default)]
    chat: Option<String>,
    profile: String,
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
            return Err("This profile was paired with another Vibe Work identity; no action ran. Reconnect the profile and select it in this chat.".into());
        }
        Ok(())
    }
    pub(super) fn check(&self, chat: &str, profile: &str) -> Result<(), String> {
        if self.storage_unavailable {
            return Err(super::profile_storage::UNREADABLE.into());
        }
        if self.ambiguous.contains_key(chat) {
            return Err("This legacy chat has several browser profiles; no action ran. Ask the user to choose a profile in this chat.".into());
        }
        if self.pending.contains_key(chat) {
            return Err("This chat's profile switch is unconfirmed; no action ran. Reconnect the old profile and retry the selection.".into());
        }
        if let Some(identity) = self.chats.get(chat) {
            self.check_owner(identity)?;
            if !super::profile_ids::same(&identity.profile, profile) {
                return Err("This chat is bound to another browser profile; no action ran. Select the intended profile in this chat before continuing.".into());
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
                return Err("This browser session could not be confirmed as belonging to this chat. Reconnect this profile before opening a new session.".into());
            }
            if !super::profile_ids::same(&identity.profile, profile) {
                return Err("This browser session is already associated with another profile. Reconnect this profile before opening a new session.".into());
            }
        }
        Ok(())
    }
}

pub(super) fn check(chrome: &ChromeControl, chat: &str, profile: &str) -> Result<(), String> {
    // Existing single-binding chats retain their exact choice, including legacy raw Chrome UUIDs.
    if let Some(selected) = chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(chat)
        && !super::profile_ids::same(selected, profile)
    {
        return Err("This chat is bound to another browser profile; no action ran. Select the intended profile in this chat before continuing.".into());
    }
    chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .check(chat, profile)
}

pub(super) fn remember(
    chrome: &ChromeControl,
    chat: &str,
    profile: &str,
    session: Option<&str>,
) -> Result<(), String> {
    check(chrome, chat, profile)?;
    let mut saved = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(session) = session {
        saved.check_session(chat, session, profile)?;
    }
    let mut next = saved.clone();
    let identity = Identity {
        chat: Some(chat.into()),
        profile: profile.into(),
        owner: next.owner.clone(),
    };
    if let Some(id) = session {
        let leases = next.leases.entry(chat.into()).or_default();
        if !leases.iter().any(|lease| lease.id == id) {
            leases.push(super::profile_selection::Release {
                id: id.into(),
                profile: profile.into(),
            });
        }
        next.sessions.insert(id.into(), identity.clone());
    }
    next.chats.insert(chat.into(), identity);
    next.save(chrome.dir())?;
    *saved = next;
    chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(chat.into(), profile.into());
    Ok(())
}

pub(super) fn forget(chrome: &ChromeControl, chat: &str) {
    let mut saved = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    saved.chats.remove(chat);
    saved.pending.remove(chat);
    saved.ambiguous.remove(chat);
    saved.leases.remove(chat);
    if let Err(reason) = saved.save(chrome.dir()) {
        tracing::warn!(%reason, "Browser binding removal could not be saved");
    }
}

/// Current selection, including a legacy single binding that has not yet opened a new task.
pub(super) fn selected(chrome: &ChromeControl, chat: &str) -> Option<String> {
    chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(chat)
        .cloned()
}
pub(super) fn generation(chrome: &ChromeControl, chat: &str) -> u64 {
    chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
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
    let mut saved = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
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
    let mut saved = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
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
    next.save(chrome.dir())?;
    *saved = next;
    chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
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
    chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(old);
    Ok(())
}

/// Migrate the helper's authenticated client-chat sessions, never a page's output or an array position.
pub(super) fn migrate(chrome: &ChromeControl, chat: &str, status: &Value) -> Result<(), String> {
    if selected(chrome, chat).is_some() {
        return Ok(());
    }
    let mut by_profile =
        std::collections::BTreeMap::<String, Vec<super::profile_selection::Release>>::new();
    for row in status["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["clientSessionId"] == chat)
    {
        let (Some(profile), Some(id)) = (row["profileId"].as_str(), row["sessionId"].as_str())
        else {
            continue;
        };
        if profile.is_empty() || id.is_empty() {
            continue;
        }
        let releases = by_profile
            .entry(super::profiles::canonical(profile))
            .or_default();
        if row["state"] != "stopped" {
            releases.push(super::profile_selection::Release {
                id: id.into(),
                profile: profile.into(),
            });
        }
    }
    if by_profile.len() == 1 {
        let Some((profile, leases)) = by_profile.into_iter().next() else {
            return Ok(());
        };
        remember(chrome, chat, &profile, None)?;
        for lease in leases {
            remember(chrome, chat, &profile, Some(&lease.id))?;
        }
    } else if by_profile.len() > 1 {
        let mut saved = chrome
            .bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut next = saved.clone();
        next.ambiguous
            .insert(chat.into(), by_profile.keys().cloned().collect());
        next.leases
            .insert(chat.into(), by_profile.into_values().flatten().collect());
        next.save(chrome.dir())?;
        *saved = next;
    }
    if let Some(profile) = selected(chrome, chat) {
        check(chrome, chat, &profile)
    } else {
        chrome
            .bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .check(chat, "")
    }
}

/// Read-back stopped receipts retire durable release obligations; mere disconnection cannot.
pub(super) fn acknowledge(chrome: &ChromeControl, rows: &[Value]) {
    let mut saved = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut next = saved.clone();
    let mut changed = false;
    for (chat, leases) in &mut next.leases {
        let before = leases.len();
        leases.retain(|lease| {
            !rows.iter().any(|row| {
                saved.check_session(chat, &lease.id, &lease.profile).is_ok()
                    && row["sessionId"] == lease.id
                    && row["profileId"]
                        .as_str()
                        .is_some_and(|id| super::profile_ids::same(id, &lease.profile))
                    && row["state"] == "stopped"
                    && row["capture"] == "off"
            })
        });
        changed |= before != leases.len();
    }
    if changed {
        if let Err(reason) = next.save(chrome.dir()) {
            tracing::warn!(%reason, "Browser release receipt could not be saved");
        } else {
            *saved = next;
        }
    }
}

/// One owner observation updates the durable identity namespace and its readable profile inventory.
pub(super) fn observe(chrome: &ChromeControl, data: &Value) {
    chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .observe_owner(data);
    super::profiles::observe(chrome, data);
}

/// Receipt identity outlives the bounded work display; only authenticated admitted sessions enter this map.
pub(super) fn session_profile(chrome: &ChromeControl, session: &str) -> Option<String> {
    let saved = chrome
        .bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if saved.storage_unavailable {
        return None;
    }
    saved
        .sessions
        .get(session)
        .map(|identity| identity.profile.clone())
}
