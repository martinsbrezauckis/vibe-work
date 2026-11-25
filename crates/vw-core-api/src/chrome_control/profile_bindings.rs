//! Durable browser bindings owned by the app. The gateway saves exact authenticated identities, never labels.
//! Helper restarts keep chat choices; owner changes and reused session IDs cannot retarget saved work.
use super::ChromeControl;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::Path};
use vw_platform::sync::LockRecover;
#[path = "profile_bindings_status.rs"]
mod status;
pub(super) use status::{
    migrate, observe, open_ended, reconcile_completed_status, session_profile,
};

#[derive(Clone, Serialize, Deserialize)]
struct Identity {
    #[serde(default)]
    chat: Option<String>,
    profile: String,
    owner: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct UncertainOpen {
    #[serde(skip)]
    observed_after: Option<tokio::time::Instant>,
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
    /// Profiles this chat deliberately left; a late unknown-ID helper row cannot reselect one.
    #[serde(default)]
    retired_profiles: HashMap<String, Vec<String>>,
    /// Chats with agent browser windows in a canonical profile not yet confirmed closed across restarts.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    unclosed_windows: HashMap<String, Vec<String>>,
    /// Chats that already warned in a canonical profile that earlier unresolvable windows may still be open.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    warned_windows: HashMap<String, Vec<String>>,
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
    pub(super) fn unclosed_windows(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.unclosed_windows
            .iter()
            .map(|(chat, profiles)| (chat.as_str(), profiles.as_slice()))
    }
    pub(super) fn warned_windows(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.warned_windows
            .iter()
            .map(|(chat, profiles)| (chat.as_str(), profiles.as_slice()))
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
    let stopping = bound.as_deref().is_some_and(|profile| {
        let work = chrome.work.lock_or_recover();
        work.releases(chat).iter().any(|lease| {
            super::profile_ids::same(&lease.profile, profile)
                && work.state_of(&lease.id, chat) == Some("stopping")
        })
    });
    let name = bound.clone().map_or_else(
        || vw_vocabulary::texts::BROWSER_PROFILE_CURRENT.to_owned(),
        |id| super::profiles::label(chrome, &id),
    );
    if stopping {
        return format!(
            "Not run: this chat's {name} browser session is still stopping; try again in a few seconds."
        );
    }
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

pub(super) fn close_candidates(chrome: &ChromeControl, chat: &str, profile: &str) -> Vec<String> {
    let saved = chrome.bindings.lock_or_recover();
    close_candidates_from(&saved, chat, profile)
}

pub(super) fn close_known(chrome: &ChromeControl, chat: &str, profile: &str) -> bool {
    if selected(chrome, chat).as_deref() == Some(profile) {
        return true;
    }
    let saved = chrome.bindings.lock_or_recover();
    saved
        .chats
        .get(chat)
        .is_some_and(|id| id.profile == profile)
        || saved
            .leases
            .get(chat)
            .is_some_and(|leases| leases.iter().any(|lease| lease.profile == profile))
        || saved
            .uncertain_opens
            .get(chat)
            .is_some_and(|open| open.profile.as_deref() == Some(profile))
        || saved
            .pending
            .get(chat)
            .is_some_and(|change| change.releases.iter().any(|lease| lease.profile == profile))
}

pub(super) fn other_firefox_hold(chrome: &ChromeControl, chat: &str) -> bool {
    let saved = chrome.bindings.lock_or_recover();
    saved.leases.iter().any(|(owner, leases)| {
        owner != chat && leases.iter().any(|lease| lease.profile == "firefox:agent")
    }) || saved.uncertain_opens.iter().any(|(owner, open)| {
        owner != chat
            && open
                .profile
                .as_deref()
                .is_none_or(|id| id == "firefox:agent")
    }) || saved.pending.iter().any(|(owner, change)| {
        owner != chat
            && change
                .releases
                .iter()
                .any(|lease| lease.profile == "firefox:agent")
    })
}

/// Remove this chat's hold on the requested browser while retaining session identities as tombstones.
pub(super) fn close_agent(chrome: &ChromeControl, chat: &str, profile: &str) -> Vec<String> {
    let mut saved = chrome.bindings.lock_or_recover();
    let ids = close_candidates_from(&saved, chat, profile);
    if saved
        .chats
        .get(chat)
        .is_some_and(|id| id.profile == profile)
    {
        saved.chats.remove(chat);
    }
    if let Some(leases) = saved.leases.get_mut(chat) {
        leases.retain(|lease| lease.profile != profile);
        if leases.is_empty() {
            saved.leases.remove(chat);
        }
    }
    if saved
        .uncertain_opens
        .get(chat)
        .is_some_and(|open| open.profile.as_deref() == Some(profile))
    {
        saved.uncertain_opens.remove(chat);
    }
    if saved
        .pending
        .get(chat)
        .is_some_and(|change| change.releases.iter().any(|lease| lease.profile == profile))
    {
        saved.pending.remove(chat);
    }
    if !saved.leases.contains_key(chat) {
        saved.ambiguous.remove(chat);
    }
    let retired = saved.retired_profiles.entry(chat.to_owned()).or_default();
    if !retired.iter().any(|id| id == profile) {
        retired.push(profile.to_owned());
    }
    if let Err(reason) = saved.save(chrome.dir()) {
        tracing::warn!(%reason, "Browser close binding could not be saved");
    }
    drop(saved);
    let mut selected = chrome.chat_profiles.lock_or_recover();
    if selected.get(chat).is_some_and(|id| id == profile) {
        selected.remove(chat);
    }
    drop(selected);
    let mut picked = chrome.picked_profiles.lock_or_recover();
    if picked.get(chat).is_some_and(|id| id == profile) {
        picked.remove(chat);
    }
    ids
}

fn close_candidates_from(saved: &Bindings, chat: &str, profile: &str) -> Vec<String> {
    let mut ids = saved
        .leases
        .get(chat)
        .into_iter()
        .flatten()
        .filter(|lease| lease.profile == profile)
        .map(|lease| lease.id.clone())
        .collect::<Vec<_>>();
    if let Some(uncertain) = saved.uncertain_opens.get(chat)
        && uncertain.profile.as_deref() == Some(profile)
        && let Some(id) = &uncertain.id
        && !ids.contains(id)
    {
        ids.push(id.clone());
    }
    ids
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
            observed_after: Some(tokio::time::Instant::now()),
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
        let refusal = match picked {
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
        };
        if let Some(bound) = selected(chrome, chat) {
            format!(
                "{refusal} To release this chat's hold, use chrome_session close_browser (profileId {bound}), then open a new browser session."
            )
        } else {
            refusal
        }
    })
}

pub(super) fn check(chrome: &ChromeControl, chat: &str, profile: &str) -> Result<(), String> {
    // A prior preference is not evidence that the uncertain start used that browser.
    if uncertain(chrome, chat) {
        return Err(vw_vocabulary::texts::BROWSER_PROFILE_OPEN_UNCONFIRMED.into());
    }
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
    saved.retired_profiles.remove(chat);
    saved.unclosed_windows.remove(chat);
    saved.warned_windows.remove(chat);
    if let Err(reason) = saved.save(chrome.dir()) {
        tracing::warn!(%reason, "Browser binding removal could not be saved");
    }
}

/// Persists that `chat` has unclosed agent windows in canonical `profile`; writes only when absent.
pub(super) fn mark_unclosed_window(chrome: &ChromeControl, chat: &str, profile: &str) {
    update_window_marks(chrome, chat, profile, Some(true), None);
}

/// Clears the persisted unclosed-window mark for `(chat, canonical profile)`; writes only when present.
#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
pub(super) fn clear_unclosed_window(chrome: &ChromeControl, chat: &str, profile: &str) {
    update_window_marks(chrome, chat, profile, Some(false), None);
}

/// Updates persisted `unclosed_windows` and `warned_windows` marks in one atomic write only on state change.
pub(super) fn sync_window_marks(
    chrome: &ChromeControl,
    chat: &str,
    profile: &str,
    clear_unclosed: bool,
    add_warned: bool,
) {
    update_window_marks(
        chrome,
        chat,
        profile,
        clear_unclosed.then_some(false),
        add_warned.then_some(true),
    );
}

fn has_canonical_entry(map: &HashMap<String, Vec<String>>, chat: &str, canonical: &str) -> bool {
    map.get(chat)
        .is_some_and(|profiles| profiles.iter().any(|seen| seen == canonical))
}

fn set_canonical_entry(
    map: &mut HashMap<String, Vec<String>>,
    chat: &str,
    canonical: &str,
    present: bool,
) {
    if present {
        let entry = map.entry(chat.into()).or_default();
        if !entry.iter().any(|seen| seen == canonical) {
            entry.push(canonical.to_owned());
        }
    } else if let Some(profiles) = map.get_mut(chat) {
        profiles.retain(|seen| seen != canonical);
        if profiles.is_empty() {
            map.remove(chat);
        }
    }
}

fn update_window_marks(
    chrome: &ChromeControl,
    chat: &str,
    profile: &str,
    unclosed: Option<bool>,
    warned: Option<bool>,
) {
    let canonical = super::profiles::canonical(profile);
    let mut saved = chrome.bindings.lock_or_recover();
    let unclosed_change = unclosed
        .is_some_and(|want| has_canonical_entry(&saved.unclosed_windows, chat, &canonical) != want);
    let warned_change = warned
        .is_some_and(|want| has_canonical_entry(&saved.warned_windows, chat, &canonical) != want);
    if !unclosed_change && !warned_change {
        return;
    }
    let mut next = saved.clone();
    if let Some(want) = unclosed {
        set_canonical_entry(&mut next.unclosed_windows, chat, &canonical, want);
    }
    if let Some(want) = warned {
        set_canonical_entry(&mut next.warned_windows, chat, &canonical, want);
    }
    match next.save(chrome.dir()) {
        Ok(()) => *saved = next,
        Err(reason) => tracing::warn!(%reason, "Unclosed window mark could not be saved"),
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
