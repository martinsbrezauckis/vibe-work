//! Chat profile bindings reconciled from the helper's status: migration, completed opens and closed windows.
use super::{Bindings, ChromeControl, LockRecover, Value, check, remember, selected};

/// Migrate the helper's authenticated client-chat sessions, never a page's output or an array position.
pub(in crate::chrome_control) fn migrate(
    chrome: &ChromeControl,
    chat: &str,
    status: &Value,
) -> Result<(), String> {
    if selected(chrome, chat).is_some() {
        return Ok(());
    }
    let mut by_profile =
        std::collections::BTreeMap::<String, Vec<super::super::profile_selection::Release>>::new();
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
        if profile.is_empty() || id.is_empty() || confirmed_ended(row) {
            continue;
        }
        // A completed session remains in the helper's status history. Its durable identity also
        // remains here after its chat binding is cleared; it must never seed that old profile again.
        let known_retired = {
            let saved = chrome.bindings.lock_or_recover();
            saved.sessions.contains_key(id)
                && !saved
                    .leases
                    .get(chat)
                    .is_some_and(|leases| leases.iter().any(|lease| lease.id == id))
        };
        if known_retired {
            continue;
        }
        let releases = by_profile
            .entry(super::super::profiles::canonical(profile))
            .or_default();
        releases.push(super::super::profile_selection::Release {
            id: id.into(),
            profile: profile.into(),
        });
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
        let mut saved = chrome.bindings.lock_or_recover();
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
        chrome.bindings.lock_or_recover().check(chat, "")
    }
}

/// A complete authenticated status retires ended session leases. Keep their exact identity records
/// so a later helper session cannot reuse an old receipt for another chat or profile.
pub(in crate::chrome_control) fn reconcile_completed_status(
    chrome: &ChromeControl,
    rows: &[Value],
    window_closed: &mut Vec<(String, String)>,
) -> bool {
    let live = chrome
        .work
        .lock_or_recover()
        .sessions()
        .into_iter()
        .filter(|(_, _, state)| !matches!(state.as_str(), "stopped" | "disconnected"))
        .map(|(id, chat, _)| (chat, id))
        .collect::<std::collections::HashSet<_>>();
    let mut saved = chrome.bindings.lock_or_recover();
    let mut next = saved.clone();
    let mut changed = false;
    next.uncertain_opens.retain(|chat, uncertain| {
        if uncertain.owner != next.owner {
            return true;
        }
        let Some(id) = uncertain.id.as_deref() else {
            return true;
        };
        let Some(row) = rows.iter().find(|row| row["sessionId"] == id) else {
            changed = true;
            return false;
        };
        let exact = row["clientSessionId"] == chat.as_str()
            && uncertain.profile.as_deref().is_none_or(|profile| {
                row["profileId"]
                    .as_str()
                    .is_some_and(|id| super::super::profile_ids::same(id, profile))
            });
        if exact && confirmed_ended(row) {
            changed = true;
            false
        } else {
            true
        }
    });
    for (chat, leases) in &mut next.leases {
        let before = leases.len();
        leases.retain(|lease| {
            if !saved.sessions.contains_key(&lease.id)
                || saved
                    .check_session(chat, &lease.id, &lease.profile)
                    .is_err()
                || live.contains(&(chat.clone(), lease.id.clone()))
            {
                return true;
            }
            match rows.iter().find(|row| row["sessionId"] == lease.id) {
                // A returned row has to be the exact released session. Stopping, unknown capture,
                // and a reused ID with another profile all keep the old binding fenced.
                Some(row) => {
                    let exact_profile = row["profileId"]
                        .as_str()
                        .is_some_and(|id| super::super::profile_ids::same(id, &lease.profile));
                    !(exact_profile && confirmed_ended(row))
                }
                // The helper's full client inventory no longer contains this session.
                None => false,
            }
        });
        changed |= before != leases.len();
    }
    let clear: Vec<_> = next
        .chats
        .iter()
        .filter(|(chat, identity)| {
            next.leases.get(*chat).is_some_and(Vec::is_empty)
                && !next.pending.contains_key(*chat)
                && !next.ambiguous.contains_key(*chat)
                && !live.iter().any(|(live_chat, _)| live_chat == *chat)
                && next.check_owner(identity).is_ok()
        })
        .map(|(chat, identity)| (chat.clone(), identity.profile.clone()))
        .collect();
    let closed = closed_windows(&saved, &clear, rows);
    for (chat, _) in &clear {
        next.chats.remove(chat);
        next.leases.remove(chat);
    }
    changed |= !clear.is_empty();
    if changed {
        *saved = next;
        {
            let mut selected = chrome.chat_profiles.lock_or_recover();
            let mut picked = chrome.picked_profiles.lock_or_recover();
            for (chat, profile) in clear {
                if selected
                    .get(&chat)
                    .is_some_and(|current| super::super::profile_ids::same(current, &profile))
                {
                    selected.remove(&chat);
                    picked.remove(&chat);
                }
            }
        }
        window_closed.extend(closed);
        if let Err(reason) = saved.save(chrome.dir()) {
            tracing::warn!(%reason, "Browser release receipt could not be saved");
        }
        return true;
    }
    false
}

/// Notices accompany only retirement of this chat's exact closed-window lease.
pub(super) fn closed_windows(
    saved: &Bindings,
    cleared: &[(String, String)],
    rows: &[Value],
) -> Vec<(String, String)> {
    cleared
        .iter()
        .filter(|(chat, profile)| {
            saved.leases.get(chat).is_some_and(|leases| {
                leases.iter().any(|lease| {
                    saved.check_session(chat, &lease.id, profile).is_ok()
                        && super::super::profile_ids::same(&lease.profile, profile)
                        && rows.iter().any(|row| {
                            row["sessionId"] == lease.id
                                && row["profileId"]
                                    .as_str()
                                    .is_some_and(|id| super::super::profile_ids::same(id, profile))
                                && row["state"] == "disconnected"
                                && row["suspensionReason"] == "WINDOW_CLOSED"
                        })
                })
            })
        })
        .cloned()
        .collect()
}

pub(super) fn confirmed_ended(row: &Value) -> bool {
    row["state"] == "stopped" && row["capture"] == "off"
        || row["state"] == "disconnected" && super::super::person_stop::connection_ended(row)
}

/// One owner observation updates the durable identity namespace and its readable profile inventory.
pub(in crate::chrome_control) fn observe(chrome: &ChromeControl, data: &Value) {
    chrome.bindings.lock_or_recover().observe_owner(data);
    super::super::profiles::observe(chrome, data);
}

/// Receipt identity outlives the bounded work display; only authenticated admitted sessions enter this map.
pub(in crate::chrome_control) fn session_profile(
    chrome: &ChromeControl,
    session: &str,
) -> Option<String> {
    let saved = chrome.bindings.lock_or_recover();
    if saved.storage_unavailable {
        return None;
    }
    saved
        .sessions
        .get(session)
        .map(|identity| identity.profile.clone())
}
