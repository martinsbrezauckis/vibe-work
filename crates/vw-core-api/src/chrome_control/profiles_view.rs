//! The paired-profile lists: the Settings view, and the agent's view and inventory.
use super::{AGENT_FIREFOX, Browser, BrowserProfile, LockRecover, ProfileState};

/// The profiles an agent may choose, default or be asked about: every paired Chrome, Edge and Safari identity. The
/// person's Firefox add-on is push-only and never takes an agent action, so no Firefox identity appears here; the
/// agent's only Firefox is the owned window (`AGENT_FIREFOX`).
pub(in crate::chrome_control) fn agent_view(shared: &crate::core::Shared) -> Vec<BrowserProfile> {
    let mut rows = view(shared);
    rows.retain(|row| Browser::from_profile(&row.profile_id) != Browser::Firefox);
    rows
}

/// `agent_view` plus the owned Firefox window while it is open, for the inventory the agent reads.
pub(in crate::chrome_control) fn agent_inventory(
    shared: &crate::core::Shared,
) -> Vec<BrowserProfile> {
    let mut rows = agent_view(shared);
    let open = cfg!(feature = "your-firefox")
        && Browser::Firefox.enabled(shared)
        && shared
            .chrome_control
            .owner
            .firefox_pairing
            .lock_or_recover()
            .0
            == super::super::PairingState::WindowOpen;
    if open {
        rows.push(BrowserProfile {
            profile_id: AGENT_FIREFOX.into(),
            browser: Browser::Firefox.id().into(),
            name: Browser::Firefox.short_name().into(),
            state: ProfileState::Connected,
            enabled: true,
            eligible: true,
            is_default: false,
            reason: None,
            account: None,
        });
    }
    rows
}

/// Every retained identity, in pairing order within each browser (the order Settings, the card and the agent show).
pub(in crate::chrome_control) fn view(shared: &crate::core::Shared) -> Vec<BrowserProfile> {
    let chrome = &shared.chrome_control;
    let profiles = chrome.profiles.lock_or_recover();
    let mut records: Vec<_> = profiles.records.iter().collect();
    records.sort_by_key(|(id, record)| (Browser::from_profile(id).id(), record.number, *id));
    let mut rows: Vec<BrowserProfile> = records
        .into_iter()
        .filter(|(id, _)| profiles.shown(id))
        .filter_map(|(id, record)| {
            let browser = Browser::from_profile(id);
            if (id.starts_with("edge:") && !cfg!(feature = "your-edge"))
                || (id.starts_with("safari:") && !cfg!(feature = "your-safari"))
                || (id.starts_with("firefox:") && !cfg!(feature = "your-firefox"))
            {
                return None;
            }
            let available = record.enabled && browser.enabled(shared);
            let same_owner = record
                .owner
                .as_ref()
                .zip(profiles.owner.as_ref())
                .is_none_or(|(a, b)| a == b);
            let state = if record.deleted || !available || !same_owner {
                ProfileState::Unavailable
            } else if profiles.live.contains(id) {
                ProfileState::Connected
            } else {
                ProfileState::Offline
            };
            Some(BrowserProfile {
                profile_id: id.clone(),
                browser: browser.id().into(),
                name: profiles.name(id),
                state,
                enabled: record.enabled,
                eligible: state == ProfileState::Connected,
                is_default: profiles.defaults.get(browser.id()) == Some(id),
                reason: if record.deleted {
                    Some(format!("Removed in {}", browser.short_name()))
                } else if !available {
                    Some("Off".into())
                } else if !same_owner {
                    Some("Pairing changed".into())
                } else {
                    None
                },
                account: profiles.account(id),
            })
        })
        .collect();
    drop(profiles);
    super::super::profile_reason::offline(chrome, &mut rows);
    rows
}

pub(in crate::chrome_control) fn optional_view(
    shared: &crate::core::Shared,
) -> Option<Vec<BrowserProfile>> {
    let rows = view(shared);
    (!rows.is_empty()).then_some(rows)
}
