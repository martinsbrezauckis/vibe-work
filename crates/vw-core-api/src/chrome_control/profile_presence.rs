//! Deleted Chrome/Edge profiles (the extension is deleted with its profile, so it cannot report this itself).
//! The frozen hello names no folder, so the only association is the profile's own random UUID found in that folder's
//! copy of the extension's private `IndexedDB`. Positive deletion evidence is that proven folder gone and missing from
//! the browser's `Local State`; anything less stays Offline. A removed identity is unusable at once, and its Settings
//! row is retired only when no chat binding, profile switch, lease or pending approval still uses it. Reads browser
//! files only, never writes them. Called from the owner status path, chat deletion and the Settings view.
use super::{agent_browser::Browser, probe, profile_ids, profiles, setup};
use crate::core::Shared;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::PoisonError,
    time::{Duration, Instant},
};

/// One file of the extension's `IndexedDB`; larger files are not this extension's few small keys.
const STORAGE_FILE_LIMIT: u64 = 8 * 1024 * 1024;
const LOCAL_STATE_LIMIT: u64 = 16 * 1024 * 1024;
/// Unproven identities are searched again at most this often unless the set of unproven identities changes.
const RESCAN: Duration = Duration::from_secs(60);

/// Owner status observed: prove folders, record deletions, clear them for live identities, then prune.
pub(super) fn reconcile(shared: &Shared) {
    let chrome = &shared.chrome_control;
    let (unproven, proven) = {
        let mut saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let unproven: BTreeSet<String> = saved
            .records
            .iter()
            .filter(|(id, record)| record.folder.is_none() && !record.removed && uuid(id).is_some())
            .map(|(id, _)| id.clone())
            .collect();
        let due = !unproven.is_empty()
            && saved
                .scanned
                .as_ref()
                .is_none_or(|(at, ids)| ids != &unproven || at.elapsed() >= RESCAN);
        if due {
            saved.scanned = Some((Instant::now(), unproven.clone()));
        }
        let proven: Vec<(String, PathBuf)> = saved
            .records
            .iter()
            .filter(|(id, record)| !record.deleted && !record.removed && !saved.is_live(id))
            .filter_map(|(id, record)| Some((id.clone(), record.folder.clone()?)))
            .collect();
        (if due { unproven } else { BTreeSet::new() }, proven)
    };
    let found = associate(chrome, &unproven);
    let deleted: Vec<String> = proven
        .into_iter()
        .filter(|(_, folder)| deleted_folder(folder))
        .map(|(id, _)| id)
        .collect();
    if !found.is_empty() || !deleted.is_empty() {
        let mut saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut next = saved.clone();
        for (id, folder) in found {
            if let Some(record) = next.records.get_mut(&id) {
                record.folder = Some(folder);
            }
        }
        for id in &deleted {
            if !next.is_live(id)
                && let Some(record) = next.records.get_mut(id)
            {
                record.deleted = true;
                *next.generations.entry(id.clone()).or_default() += 1;
                next.defaults.retain(|_, selected| selected != id);
            }
        }
        match next.save(chrome.dir()) {
            Ok(()) => *saved = next,
            Err(reason) => tracing::warn!(%reason, "Browser profile removal could not be saved"),
        }
    }
    // Open cards stay until the person answers them (any action is then refused by `profiles::check`); the row is
    // pruned once they settle. The generation bump above fences late replies for this identity.
    prune(shared);
}

/// Retire removed identities that nothing uses any more, exactly like the person's own Remove leaves them: a
/// `removed` tombstone, so the helper's status (which still lists its stale pairing as offline) cannot bring the row
/// back, and a later native pairing approval may still re-enable it (`profiles::reapprove`). Cheap: no file is read.
pub(super) fn prune(shared: &Shared) {
    let chrome = &shared.chrome_control;
    let candidates: Vec<String> = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .records
        .iter()
        .filter(|(_, record)| record.deleted && !record.removed)
        .map(|(id, _)| id.clone())
        .collect();
    let unused: Vec<String> = candidates
        .into_iter()
        .filter(|id| !in_use(shared, id))
        .collect();
    if unused.is_empty() {
        return;
    }
    let mut saved = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut next = saved.clone();
    for id in &unused {
        if let Some(record) = next.records.get_mut(id)
            && record.deleted
        {
            record.removed = true;
            next.defaults.retain(|_, selected| selected != id);
        }
    }
    match next.save(chrome.dir()) {
        Ok(()) => *saved = next,
        Err(reason) => tracing::warn!(%reason, "Removed browser profile could not be forgotten"),
    }
}

/// A chat binding, an unconfirmed switch or release, or an open approval still names this identity.
fn in_use(shared: &Shared, id: &str) -> bool {
    let chrome = &shared.chrome_control;
    let same = |other: &str| profile_ids::same(other, id);
    if chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .any(|profile| same(profile))
    {
        return true;
    }
    {
        let bindings = chrome
            .bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if bindings.chats().values().any(|profile| same(profile))
            || bindings.pending.values().any(|change| same(&change.target))
            || bindings
                .leases
                .values()
                .flatten()
                .any(|lease| same(&lease.profile))
        {
            return true;
        }
    }
    chrome.owner.approvals.waiting(id)
        || crate::ask::tool_calls::browser_profile_waiting(shared, id)
}

/// The UUID the extension generated (`crypto.randomUUID`, lowercase) for a Chrome or Edge identity.
fn uuid(id: &str) -> Option<&str> {
    let browser = Browser::from_profile(id);
    if !matches!(browser, Browser::Chrome | Browser::Edge) {
        return None;
    }
    id.split_once(':')
        .map(|(_, uuid)| uuid)
        .filter(|uuid| super::enroll::uuid_shape(uuid))
}

/// Each identity whose UUID is in exactly one profile folder's extension `IndexedDB`. A compacted `LevelDB` table may be
/// compressed, so a miss proves nothing; a hit on a random 122-bit UUID is the association.
fn associate(chrome: &super::ChromeControl, ids: &BTreeSet<String>) -> BTreeMap<String, PathBuf> {
    let mut hits: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    if ids.is_empty() {
        return BTreeMap::new();
    }
    for browser in [Browser::Chrome, Browser::Edge] {
        let wanted: Vec<(&String, &str)> = ids
            .iter()
            .filter(|id| Browser::from_profile(id) == browser)
            .filter_map(|id| Some((id, uuid(id)?)))
            .collect();
        if wanted.is_empty() {
            continue;
        }
        let name = if browser == Browser::Edge {
            "Microsoft Edge"
        } else {
            "Google Chrome"
        };
        let extension = &chrome.config.extension_id;
        for data in setup::user_data(&chrome.config, chrome.dir(), name) {
            for folder in probe::profiles_for(&chrome.config, &data, extension) {
                let files = storage(&folder, extension);
                for (id, uuid) in &wanted {
                    if files.iter().any(|bytes| contains(bytes, uuid.as_bytes())) {
                        hits.entry((*id).clone()).or_default().push(folder.clone());
                    }
                }
            }
        }
    }
    hits.into_iter()
        .filter_map(|(id, folders)| match folders.as_slice() {
            [one] => Some((id, one.clone())),
            _ => None,
        })
        .collect()
}

/// The extension's private `IndexedDB` files in one profile folder, regular files only. The extension keeps its UUID
/// in `IndexedDB` (`chrome-control-private`, the extension's `adapter/identity.ts`), not in `chrome.storage.local`.
/// Chromium 156 writes one SQLite file per database under `IndexedDB/chrome-extension_<id>_0/` (checked on
/// 2026-11-10 with an unpacked extension running that exact code: the UUID is stored as plain bytes); earlier
/// releases use the `LevelDB` folder `IndexedDB/chrome-extension_<id>_0.indexeddb.leveldb/`. Both are read.
fn storage(folder: &Path, extension: &str) -> Vec<Vec<u8>> {
    let root = folder.join("IndexedDB");
    let origin = format!("chrome-extension_{extension}_0");
    [
        root.join(&origin),
        root.join(format!("{origin}.indexeddb.leveldb")),
    ]
    .iter()
    .filter(|dir| std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir()))
    .filter_map(|dir| std::fs::read_dir(dir).ok())
    .flat_map(|entries| entries.flatten().take(64))
    .filter_map(|entry| probe::read_regular(&entry.path(), STORAGE_FILE_LIMIT))
    .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Positive evidence only: the browser's data folder is still there and readable, the proven profile folder is not,
/// and `Local State` no longer lists it. A missing data folder, an unreadable state or a listed folder prove nothing.
fn deleted_folder(folder: &Path) -> bool {
    let (Some(data), Some(name)) = (folder.parent(), folder.file_name()) else {
        return false;
    };
    if !std::fs::symlink_metadata(data).is_ok_and(|meta| meta.is_dir())
        || !std::fs::symlink_metadata(folder)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return false;
    }
    let Some(state) = probe::read_regular(&data.join("Local State"), LOCAL_STATE_LIMIT)
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return false;
    };
    state["profile"]["info_cache"]
        .as_object()
        .is_some_and(|cache| !cache.contains_key(&*name.to_string_lossy()))
}

/// The person's next step for a chat still bound to a removed identity.
pub(super) fn removed_message(chrome: &super::ChromeControl, id: &str) -> String {
    format!(
        "{} was removed; no action ran. Choose another profile in this chat or reconnect this profile.",
        profiles::label(chrome, id)
    )
}

#[cfg(all(test, feature = "your-edge"))]
pub(super) fn hold_for_test(shared: &std::sync::Arc<Shared>, id: &str) -> Option<impl Sized> {
    crate::ask::tool_calls::open_browser_card_for_test(shared, "s", id)
}
