//! Why a paired Chrome/Edge profile is offline, from Chrome's own record of the extension in that profile's proven
//! folder (`profile_presence`). Called by the Settings profile view and the browser card's state. Reads browser
//! files only, outside the profile lock; a folder that is not proven or not readable gives no reason.
use super::{BrowserProfile, ChromeControl, ProfileState, agent_browser::Browser, probe};
use std::path::PathBuf;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

/// The extension is off in the row's own profile: the person turns it on there.
pub(super) const EXTENSION_OFF: &str = "Extension off in this profile";
/// The extension is on, so the profile runs it only while one of its windows is open.
pub(super) const NO_WINDOW: &str = "No window of this profile open";

/// Fills the reason of each offline row whose folder is proven.
pub(super) fn offline(chrome: &ChromeControl, rows: &mut [BrowserProfile]) {
    let folders: Vec<(usize, PathBuf)> = {
        let saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        rows.iter()
            .enumerate()
            .filter(|(_, row)| row.state == ProfileState::Offline && row.reason.is_none())
            .filter_map(|(index, row)| {
                Some((index, saved.records.get(&row.profile_id)?.folder.clone()?))
            })
            .collect()
    };
    // Without the helper no profile can connect, so an open window would not show either: only "off" is then sure.
    let helper_up = !chrome
        .owner
        .tools
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty();
    for (index, folder) in folders {
        rows[index].reason =
            match probe::folder(&chrome.config, &folder, &chrome.config.extension_id) {
                probe::Probe::Off | probe::Probe::AwaitingEnable => Some(EXTENSION_OFF.into()),
                probe::Probe::NotOff if helper_up => Some(NO_WINDOW.into()),
                probe::Probe::NotOff | probe::Probe::Unknown => None,
            };
    }
}

/// How long after a browser's last paired profile went offline its card keeps the "Turn on" guidance: the browser
/// writes an extension the person turned off to its preferences only some seconds later.
pub(super) const PREFERENCES_DELAY: Duration = Duration::from_secs(15);

/// Owner status observed: records since when each browser has had no live paired profile.
pub(super) fn observe(chrome: &ChromeControl) {
    let live: Vec<Browser> = {
        let saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        saved
            .records
            .keys()
            .filter(|id| saved.is_live(id))
            .map(|id| Browser::from_profile(id))
            .collect()
    };
    let mut since = chrome
        .offline_since
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for browser in [Browser::Chrome, Browser::Edge] {
        if live.contains(&browser) {
            since.remove(browser.id());
        } else {
            since.entry(browser.id()).or_insert_with(Instant::now);
        }
    }
}

/// What the files say once every paired profile of a browser has been offline longer than `PREFERENCES_DELAY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Window {
    /// A proven paired folder says the extension is on: no window of a paired profile is open.
    Closed,
    /// No paired folder can be read (macOS, or a folder not proven yet): a window may be closed or the profile may
    /// wait for the person's approval; the extension is not known to be off.
    Unknown,
}

/// `None` while the browser's preferences may still be catching up, or when a paired folder says the extension is off.
pub(super) fn window(chrome: &ChromeControl, browser: Browser) -> Option<Window> {
    let settled = chrome
        .offline_since
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(browser.id())
        .is_some_and(|since| since.elapsed() >= PREFERENCES_DELAY);
    if !settled {
        return None;
    }
    let folders: Vec<PathBuf> = {
        let saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        saved
            .records
            .iter()
            .filter(|(id, record)| {
                Browser::from_profile(id) == browser && !record.removed && !record.deleted
            })
            .filter_map(|(_, record)| record.folder.clone())
            .collect()
    };
    let read: Vec<probe::Probe> = folders
        .iter()
        .map(|folder| probe::folder(&chrome.config, folder, &chrome.config.extension_id))
        .collect();
    if read.contains(&probe::Probe::NotOff) {
        Some(Window::Closed)
    } else if read.iter().all(|probe| *probe == probe::Probe::Unknown) {
        Some(Window::Unknown)
    } else {
        None
    }
}

/// No window of a paired profile is open and a proven paired folder says the extension is on.
#[cfg(all(test, feature = "your-edge"))]
pub(super) fn no_window(chrome: &ChromeControl, browser: Browser) -> bool {
    window(chrome, browser) == Some(Window::Closed)
}

/// The reconnecting card's later guidance and the "Not now" card, for a paired browser's card.
pub(super) fn window_card(chrome: &ChromeControl, view: &mut super::AddonView, browser: Browser) {
    match window(chrome, browser) {
        Some(Window::Closed) => super::addon::window_closed_for(view, browser),
        Some(Window::Unknown) => super::addon::window_or_approval_for(view, browser),
        None => {}
    }
    super::pairing_approval::not_now_card(chrome, view, browser);
}

/// A paired browser's reconnecting card (`addon::reconnecting_for`), then `window_card`.
pub(super) fn reconnecting(
    shared: &crate::core::Shared,
    view: &mut super::AddonView,
    not_removed: bool,
    browser: Browser,
) {
    super::addon::reconnecting_for(view, browser.reconnectable(shared) && not_removed, browser);
    window_card(&shared.chrome_control, view, browser);
}

#[cfg(all(test, feature = "your-edge"))]
pub(super) fn offline_for_test(chrome: &ChromeControl, browser: Browser, ago: Duration) {
    let at = Instant::now().checked_sub(ago).unwrap_or_else(Instant::now);
    chrome
        .offline_since
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(browser.id(), at);
}
