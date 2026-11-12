//! Legacy profile history migration. The frozen hello has no filesystem association, so old inferred folders
//! are discarded. Installation may retain observed IDs, but cannot use a directory to choose an account.
use super::{ChromeControl, agent_browser::Browser};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct History(BTreeMap<String, Option<PathBuf>>);

impl History {
    #[cfg(all(test, feature = "your-edge"))]
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
    let mut history: History = record(chrome, browser)
        .and_then(|path| super::probe::read_regular(&path, 64 * 1024))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for folder in history.0.values_mut() {
        *folder = None;
    }
    history
}

/// Preserve observed IDs without guessing an association from counts, names or directory order.
pub(super) fn observe(chrome: &ChromeControl, browser: Browser) {
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
