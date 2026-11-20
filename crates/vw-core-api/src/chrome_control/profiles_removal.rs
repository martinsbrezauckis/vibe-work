//! Removing, retiring and re-approving paired browser profiles, and what removal leaves for the agent.
use super::{AGENT_FIREFOX, Browser, ChromeControl, LockRecover, Profiles, Value, canonical};

/// Every pairing of `browser` was removed in Browser settings (at least one record, none kept). A retired identity
/// (`Profiles::retired`) is no pairing kept: it is never shown, and its folder holds a newer identity. A display fact
/// for the card: the extension may still be on, and only a new pairing brings the browser back.
pub(in crate::chrome_control) fn all_removed(chrome: &ChromeControl, browser: Browser) -> bool {
    let saved = chrome.profiles.lock_or_recover();
    let mut mine = saved
        .records
        .iter()
        .filter(|(id, _)| browser.owns_profile(id))
        .peekable();
    mine.peek().is_some() && mine.all(|(id, record)| record.removed || saved.retired(id))
}

/// In a helper status, marks each profile the person removed in Settings (and has not paired again, `reapprove`) as not
/// connected. Its extension may still reach the helper, but it is not its browser's connection: the browser pairing
/// states read from this status (the card and the agent) never say Connected because of it.
pub(in crate::chrome_control) fn removed_not_connected(chrome: &ChromeControl, data: &mut Value) {
    let Some(rows) = data.get_mut("profiles").and_then(Value::as_array_mut) else {
        return;
    };
    let saved = chrome.profiles.lock_or_recover();
    for row in rows {
        let removed = row["profileId"].as_str().is_some_and(|id| {
            saved
                .records
                .get(&canonical(id))
                .is_some_and(|record| record.removed)
        });
        if removed {
            row["connected"] = Value::Bool(false);
        }
    }
}

/// The older identities proven to be the same browser profile as `id`: a Chrome or Edge identity in `id`'s folder with
/// an earlier pairing, not removed and not connected now (`Profiles::retired`). They are never shown, yet the helper
/// keeps their approval, so Remove of `id` unpairs them too: no approved pairing of that profile stays without a row.
/// A Safari or Firefox identity that `id` only replaced (`replaced_by`) is not proven to be the same profile; it may be
/// a second real profile, so it keeps its pairing and shows again when it connects.
pub(in crate::chrome_control) fn retired_by(chrome: &ChromeControl, id: &str) -> Vec<String> {
    let saved = chrome.profiles.lock_or_recover();
    let Some(newer) = saved.records.get(id) else {
        return Vec::new();
    };
    let browser = Browser::from_profile(id);
    saved
        .records
        .iter()
        .filter(|(other, record)| {
            other.as_str() != id
                && other.as_str() != AGENT_FIREFOX
                && browser.owns_profile(other)
                && !record.removed
                && saved.retired(other)
                && newer.folder.is_some()
                && record.folder == newer.folder
                && (record.number, other.as_str()) < (newer.number, id)
        })
        .map(|(other, _)| other.clone())
        .collect()
}

/// A newer identity of the same browser profile replaced this one (`Profiles::retired`).
pub(in crate::chrome_control) fn retired(chrome: &ChromeControl, id: &str) -> bool {
    chrome.profiles.lock_or_recover().retired(&canonical(id))
}

/// The Remove hint when the removal could not be saved (`{browser}` as in `Browser::words`).
pub(in crate::chrome_control) const REMOVAL_NOT_SAVED: &str = "Vibe Work could not save this removal: the {browser} profiles may show again after Vibe Work restarts. Remove them again then.";

/// The person removed a whole browser in Settings: every identity of it is removed like a single profile's Remove
/// (record kept, off, no default), so it never returns as an Offline profile; pairing it again brings it back.
/// The removal stays in memory even when it cannot be saved (the helper side is already unpaired, so this run must
/// not show those identities again); a failed save is only logged ([`remove_browser_saved`] reports it).
pub(in crate::chrome_control) fn remove_browser(chrome: &ChromeControl, browser: Browser) {
    let _ = remove_browser_saved(chrome, browser);
}

/// [`remove_browser`] with the failed save as an error, so the caller can tell the person that a restart may bring
/// the removed identities back ([`REMOVAL_NOT_SAVED`]). An inventory that was unreadable when Vibe Work started is not
/// written (`Profiles::save` refuses it, leaving the file as it is), so that removal is unsaved too and says so.
pub(in crate::chrome_control) fn remove_browser_saved(
    chrome: &ChromeControl,
    browser: Browser,
) -> Result<(), String> {
    let mut saved = chrome.profiles.lock_or_recover();
    let mut next = saved.clone();
    for (id, record) in &mut next.records {
        if browser.owns_profile(id) {
            record.removed = true;
            record.enabled = false;
        }
    }
    next.defaults.remove(browser.id());
    next.live.retain(|id| !browser.owns_profile(id));
    let written = next.save(chrome.dir());
    if let Err(reason) = &written {
        tracing::warn!(%reason, "Browser profile inventory could not be saved");
    }
    *saved = next;
    written
}

/// The last browser connection was removed and its folder deleted: nothing of the old inventory may be saved again.
pub(in crate::chrome_control) fn forget_all(chrome: &ChromeControl) {
    let mut saved = chrome.profiles.lock_or_recover();
    if saved.storage_unavailable {
        return;
    }
    // Fences for in-flight work keep counting up, so an earlier task can never match a later generation.
    let generations = std::mem::take(&mut saved.generations);
    *saved = Profiles {
        generations,
        ..Profiles::default()
    };
}

/// A removed pin can request a fresh native ceremony; an identity switched Off cannot.
pub(in crate::chrome_control) fn pairing_allowed(chrome: &ChromeControl, id: &str) -> bool {
    let saved = chrome.profiles.lock_or_recover();
    !saved.storage_unavailable
        && saved
            .records
            .get(&canonical(id))
            .is_none_or(|record| record.enabled || record.removed)
}

/// Called only after the person approves the native pairing dialog, before accepting the new signed key.
pub(in crate::chrome_control) fn reapprove(chrome: &ChromeControl, id: &str) -> Result<(), String> {
    let mut saved = chrome.profiles.lock_or_recover();
    let mut next = saved.clone();
    let id = canonical(id);
    if let Some(record) = next.records.get_mut(&id)
        && record.removed
    {
        record.removed = false;
        record.enabled = true;
        record.owner.clone_from(&next.owner);
        next.live.remove(&id);
    }
    next.save(chrome.dir())?;
    *saved = next;
    Ok(())
}
