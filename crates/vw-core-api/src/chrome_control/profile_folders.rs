//! App-owned associations between observed paired IDs and unambiguous extension folders. Installation uses
//! these only for diagnosis and launching; they grant no pairing authority and never read extension identities.
use super::{ChromeControl, agent_browser::Browser};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct History(BTreeMap<String, Option<PathBuf>>);

impl History {
    pub(super) fn contains(&self, path: &std::path::Path) -> bool {
        self.0.values().flatten().any(|folder| folder == path)
    }
}

fn record(chrome: &ChromeControl, browser: Browser) -> Option<PathBuf> {
    let state = chrome.dir()?;
    match browser {
        Browser::Chrome => Some(state.join("profile-folders.json")),
        Browser::Edge => Some(state.join("edge/profile-folders.json")),
        Browser::Safari | Browser::Firefox => None,
    }
}

pub(super) fn load(chrome: &ChromeControl, browser: Browser) -> History {
    record(chrome, browser)
        .and_then(|path| super::probe::read_regular(&path, 64 * 1024))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// A sole new connected ID and sole unclaimed eligible folder establish an association. Multiple unknown
/// folders remain ambiguous; never assign a random extension ID by directory order or browser display name.
pub(super) fn observe(chrome: &ChromeControl, browser: Browser, eligible: &[PathBuf]) {
    let Some(record) = record(chrome, browser) else {
        return;
    };
    let mut history = load(chrome, browser);
    let before = serde_json::to_vec(&history).ok();
    let live: Vec<_> = chrome
        .owner
        .connected_profiles
        .snapshot()
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| browser.owns_profile(id))
        .collect();
    if live.is_empty() {
        return;
    }
    for id in &live {
        history.0.entry(id.clone()).or_default();
    }
    let unknown: Vec<_> = live.iter().filter(|id| history.0[*id].is_none()).collect();
    let folders: Vec<_> = eligible
        .iter()
        .filter(|path| !history.contains(path))
        .collect();
    if let ([id], [folder]) = (unknown.as_slice(), folders.as_slice()) {
        history.0.insert((*id).clone(), Some((*folder).clone()));
    }
    let Ok(bytes) = serde_json::to_vec(&history) else {
        return;
    };
    if before.as_ref() == Some(&bytes) {
        return;
    }
    let Some(parent) = record.parent() else {
        return;
    };
    if let Err(reason) = std::fs::create_dir_all(parent)
        .map_err(|reason| reason.to_string())
        .and_then(|()| {
            vw_platform::replace_private(&record, &bytes).map_err(|reason| reason.to_string())
        })
    {
        tracing::debug!(%reason, "paired browser folders could not be remembered");
    }
}

pub(super) fn forget(chrome: &ChromeControl, browser: Browser) {
    if let Some(record) = record(chrome, browser) {
        let _ = std::fs::remove_file(record);
    }
}
