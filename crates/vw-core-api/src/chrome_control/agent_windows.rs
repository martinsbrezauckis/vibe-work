//! The windows each chat's agent opened in an external Chrome, Edge or Safari profile, for `close_browser`
//! (`agent_close`). Bounded both ways: per chat and in the number of chats. A deleted chat's entry is removed.
//! Memory only: after an app restart the helper no longer owns the earlier windows' tabs, so a saved window would gain
//! nothing, and a saved window number can name a person's window in the next browser (DBK17S-01).
//!
//! Chrome numbers windows per browser session and reuses a number after the browser restarts, so a window number alone
//! never identifies a window. Every remembered window therefore carries the browser incarnation (the helper's
//! `browserSessionId` of the profile) it was opened in, and `close_browser` nominates it only while the helper reports
//! the same incarnation for that profile. When `observe` drops remembered windows on an incarnation change, it keeps a
//! bounded per-`(chat, profile)` mark so the next `close_browser` still warns that earlier tabs may remain (DBK17T-01).
use serde_json::Value;
use std::collections::HashMap;

/// The most chats whose windows (or dropped marks) are remembered; when another chat needs room, the chat remembered
/// first is dropped.
const MAX_CHATS: usize = 256;
/// The most agent windows (or dropped profile marks) remembered per chat; the oldest is forgotten first.
const MAX_PER_CHAT: usize = 16;
/// The most profiles whose browser incarnation is kept (a person's browser has a handful).
const MAX_PROFILES: usize = 64;

/// A remembered window: its profile, the browser incarnation it was opened in, and its number there.
struct Held {
    profile: String,
    incarnation: String,
    window: u64,
}

/// Chats in the order their first window was remembered, each with its windows oldest first, per-chat canonical
/// profiles whose earlier windows were dropped on an incarnation mismatch (`dropped`), and the browser incarnation the
/// app last saw for each profile (the helper's status, `observe`).
#[derive(Default)]
pub(super) struct AgentWindows {
    chats: Vec<(String, Vec<Held>)>,
    dropped: Vec<(String, Vec<String>)>,
    incarnations: HashMap<String, String>,
}

/// Returns `chat`'s list inside `list`, inserting a new entry at the back and evicting the oldest chat at `MAX_CHATS`.
fn chat_slot<'a, T>(list: &'a mut Vec<(String, Vec<T>)>, chat: &str) -> &'a mut Vec<T> {
    let at = if let Some(at) = list.iter().position(|(id, _)| id == chat) {
        at
    } else {
        if list.len() >= MAX_CHATS {
            list.remove(0);
        }
        list.push((chat.to_owned(), Vec::new()));
        list.len() - 1
    };
    &mut list[at].1
}

impl AgentWindows {
    /// Reads the browser incarnation of each profile row of a helper status. A profile whose incarnation changed is a
    /// restarted browser: every chat's remembered windows of it are dropped (their numbers may name other windows now).
    pub(super) fn observe(&mut self, status: &Value) {
        for row in status["profiles"].as_array().into_iter().flatten() {
            let (Some(profile), Some(incarnation)) = (
                row["profileId"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 128),
                row["browserSessionId"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 128),
            ) else {
                continue;
            };
            let profile = super::profiles::canonical(profile);
            let known = self.incarnations.get(&profile).cloned();
            if known.as_deref() == Some(incarnation) {
                continue;
            }
            if known.is_some() {
                self.drop_profile(&profile);
            } else if self.incarnations.len() >= MAX_PROFILES {
                continue;
            }
            self.incarnations.insert(profile, incarnation.to_owned());
        }
    }

    /// Remembers a window of `profile` that `chat`'s agent opened, bound to the profile's current browser incarnation;
    /// false when it was remembered already or when no incarnation is known (nothing is remembered without one).
    pub(super) fn add(&mut self, chat: &str, profile: &str, window: u64) -> bool {
        let canonical = super::profiles::canonical(profile);
        let Some(incarnation) = self.incarnations.get(&canonical).cloned() else {
            return false;
        };
        let seen = chat_slot(&mut self.chats, chat);
        if seen.iter().any(|held| {
            held.profile == profile && held.incarnation == incarnation && held.window == window
        }) {
            return false;
        }
        seen.push(Held {
            profile: profile.to_owned(),
            incarnation,
            window,
        });
        if seen.len() > MAX_PER_CHAT {
            seen.remove(0);
        }
        true
    }

    /// `chat`'s remembered windows in `profile`, newest first.
    pub(super) fn of(&self, chat: &str, profile: &str) -> Vec<u64> {
        self.chats
            .iter()
            .find(|(id, _)| id == chat)
            .map(|(_, seen)| {
                seen.iter()
                    .rev()
                    .filter(|held| held.profile == profile)
                    .map(|held| held.window)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The windows of `chat` in `profile` that may be nominated now, newest first: those opened in the browser
    /// incarnation `live` that the helper reports for the profile. The others (another incarnation, or none reported)
    /// are dropped, their numbers can name a person's window now; the flag says that some were dropped here or already
    /// dropped by `observe` (`DBK17T-01`).
    pub(super) fn nominate(
        &mut self,
        chat: &str,
        profile: &str,
        live: Option<&str>,
    ) -> (Vec<u64>, bool) {
        let canonical = super::profiles::canonical(profile);
        let had_dropped = self.has_dropped(chat, profile);
        let Some(at) = self.chats.iter().position(|(id, _)| id == chat) else {
            return (Vec::new(), had_dropped);
        };
        let (dropped, empty) = {
            let seen = &mut self.chats[at].1;
            let before = seen.len();
            seen.retain(|held| held.profile != profile || Some(held.incarnation.as_str()) == live);
            (seen.len() != before, seen.is_empty())
        };
        if empty {
            self.chats.remove(at);
        }
        if dropped {
            self.mark_dropped(chat, &canonical);
        }
        (self.of(chat, profile), dropped || had_dropped)
    }

    /// Forgets the windows of `chat` in `profile` that `done` names; true when any was forgotten.
    pub(super) fn forget(&mut self, chat: &str, profile: &str, done: impl Fn(u64) -> bool) -> bool {
        let Some(at) = self.chats.iter().position(|(id, _)| id == chat) else {
            return false;
        };
        let seen = &mut self.chats[at].1;
        let before = seen.len();
        seen.retain(|held| held.profile != profile || !done(held.window));
        let changed = seen.len() != before;
        if seen.is_empty() {
            self.chats.remove(at);
        }
        changed
    }

    /// Whether `chat` had remembered windows in `profile` dropped on a browser incarnation change and not yet reported
    /// by a live `close_browser`.
    pub(super) fn has_dropped(&self, chat: &str, profile: &str) -> bool {
        let canonical = super::profiles::canonical(profile);
        self.dropped
            .iter()
            .find(|(id, _)| id == chat)
            .is_some_and(|(_, profiles)| profiles.iter().any(|seen| seen == &canonical))
    }

    /// True if `chat` still retains any remembered window or dropped mark under `profile`'s canonical ID.
    pub(super) fn has_canonical(&self, chat: &str, profile: &str) -> bool {
        if self.has_dropped(chat, profile) {
            return true;
        }
        let canonical = super::profiles::canonical(profile);
        self.chats
            .iter()
            .find(|(id, _)| id == chat)
            .is_some_and(|(_, seen)| {
                seen.iter()
                    .any(|held| super::profiles::canonical(&held.profile) == canonical)
            })
    }

    /// Clears the dropped-window mark of `(chat, profile)` after `close_browser` answers; true when one was present.
    pub(super) fn clear_dropped(&mut self, chat: &str, profile: &str) -> bool {
        let canonical = super::profiles::canonical(profile);
        let Some(at) = self.dropped.iter().position(|(id, _)| id == chat) else {
            return false;
        };
        let profiles = &mut self.dropped[at].1;
        let before = profiles.len();
        profiles.retain(|seen| seen != &canonical);
        let changed = profiles.len() != before;
        if profiles.is_empty() {
            self.dropped.remove(at);
        }
        changed
    }

    /// Seeds per-(chat, canonical profile) dropped marks loaded on app start so the next `close_browser`
    /// warns that earlier-run agent windows may still be open.
    pub(super) fn seed_dropped<'a>(
        &mut self,
        marks: impl IntoIterator<Item = (&'a str, &'a [String])>,
    ) {
        for (chat, profiles) in marks {
            for profile in profiles {
                self.mark_dropped(chat, &super::profiles::canonical(profile));
            }
        }
    }

    /// Forgets everything of a chat that was deleted; true when it had windows or dropped marks.
    pub(super) fn remove_chat(&mut self, chat: &str) -> bool {
        let before_chats = self.chats.len();
        self.chats.retain(|(id, _)| id != chat);
        let before_dropped = self.dropped.len();
        self.dropped.retain(|(id, _)| id != chat);
        self.chats.len() != before_chats || self.dropped.len() != before_dropped
    }

    /// Forgets every chat's windows of canonical `profile` (a browser that restarted) and marks each affected chat so
    /// its next `close_browser` still warns that earlier tabs may remain.
    fn drop_profile(&mut self, profile: &str) {
        let mut affected = Vec::new();
        for (chat, seen) in &mut self.chats {
            let before = seen.len();
            seen.retain(|held| super::profiles::canonical(&held.profile) != profile);
            if seen.len() != before {
                affected.push(chat.clone());
            }
        }
        self.chats.retain(|(_, seen)| !seen.is_empty());
        for chat in affected {
            self.mark_dropped(&chat, profile);
        }
    }

    fn mark_dropped(&mut self, chat: &str, canonical_profile: &str) {
        let profiles = chat_slot(&mut self.dropped, chat);
        if profiles.iter().any(|seen| seen == canonical_profile) {
            return;
        }
        profiles.push(canonical_profile.to_owned());
        if profiles.len() > MAX_PER_CHAT {
            profiles.remove(0);
        }
    }
}
