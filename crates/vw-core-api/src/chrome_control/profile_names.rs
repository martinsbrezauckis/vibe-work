//! The browser's own names for paired Chrome and Edge profiles: the name its profile picker shows and the signed-in
//! account its profile menu shows, read from the user data folder's `Local State` (`profile.info_cache.<folder>`). A browser does not tell its
//! extension which profile it runs in, so the folder comes from `profile_presence`, which finds the identity's own
//! random UUID in that folder's copy of the extension storage. Reads browser files only, never writes them; a
//! missing, locked or malformed file leaves the saved name unchanged. A `Local State` is read again when it changes
//! or when a folder in it is newly proven. Called from the owner status path.
use super::ChromeControl;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::PoisonError,
    time::SystemTime,
};

/// `Local State` holds every profile's settings summary; real files are well under this.
const LOCAL_STATE_LIMIT: u64 = 16 * 1024 * 1024;
/// Longer names or accounts are not something a profile picker shows.
const TEXT_LIMIT: usize = 120;

/// A `Local State` file's modification time and size: a rename in the browser rewrites it.
type Stamp = (SystemTime, u64);
/// One read `Local State`: its stamp and each folder's entry.
type State = (Stamp, BTreeMap<String, Entry>);
/// When a `Local State` was last read, and each identity with its proven folder that read named.
pub(super) type Read = (Stamp, BTreeSet<(String, PathBuf)>);

/// What the browser's profile picker shows for one profile folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    /// The profile's name as the picker shows it: a policy's enterprise label, else the local name, else the
    /// signed-in account's own name when the local name is empty.
    pub(super) name: Option<String>,
    /// The signed-in account (its email), shown under the name on screen; agent text carries only its domain.
    pub(super) account: Option<String>,
}

fn text(value: &serde_json::Value) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty() && text.chars().count() <= TEXT_LIMIT && !text.chars().any(char::is_control))
        .then(|| text.to_owned())
}

/// Each profile folder's entry in one `Local State` file; `None` when the file is not a browser's `Local State`.
pub(super) fn parse(bytes: &[u8]) -> Option<BTreeMap<String, Entry>> {
    let state: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let cache = state.get("profile")?.get("info_cache")?.as_object()?;
    Some(
        cache
            .iter()
            .map(|(folder, info)| {
                let name = text(&info["enterprise_label"])
                    .or_else(|| text(&info["name"]))
                    .or_else(|| text(&info["gaia_name"]));
                let account = text(&info["user_name"]);
                (folder.clone(), Entry { name, account })
            })
            .collect(),
    )
}

/// The `Local State` of one user data folder, with the file stamp it was read at.
fn read(data: &Path) -> Option<State> {
    let path = data.join("Local State");
    let meta = std::fs::symlink_metadata(&path).ok()?;
    let stamp = (meta.modified().ok()?, meta.len());
    let bytes = super::probe::read_regular(&path, LOCAL_STATE_LIMIT)?;
    Some((stamp, parse(&bytes)?))
}

/// What the browser shows for one profile folder, read from its user data folder's `Local State` now.
pub(super) fn folder_entry(folder: &Path) -> Option<Entry> {
    let (_, mut entries) = read(folder.parent()?)?;
    entries.remove(folder.file_name()?.to_str()?)
}

/// Re-read the names of every identity with a proven folder whose `Local State` changed since the last read, or
/// that this file was not read for. Returns whether a shown name or account changed.
pub(super) fn refresh(chrome: &ChromeControl) -> bool {
    let folders: Vec<(String, PathBuf)> = {
        let saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        saved
            .records
            .iter()
            .filter(|(_, record)| !record.removed && !record.deleted)
            .filter_map(|(id, record)| Some((id.clone(), record.folder.clone()?)))
            .collect()
    };
    let parents: BTreeSet<PathBuf> = folders
        .iter()
        .filter_map(|(_, folder)| folder.parent().map(Path::to_path_buf))
        .filter(|data| super::browser_data::readable(chrome, data))
        .collect();
    let mut read_now = Vec::new();
    {
        let saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for data in parents {
            let Some(stamp) = std::fs::symlink_metadata(data.join("Local State"))
                .ok()
                .and_then(|meta| Some((meta.modified().ok()?, meta.len())))
            else {
                continue;
            };
            let read = saved.local_states.get(&data);
            // Covered per identity: an identity newly proven in a folder already read (the extension was installed
            // again in that profile) needs its name although `Local State` did not change.
            let unread = folders.iter().any(|(id, folder)| {
                folder.parent() == Some(data.as_path())
                    && read
                        .is_none_or(|(_, covered)| !covered.contains(&(id.clone(), folder.clone())))
            });
            if read.is_none_or(|(at, _)| *at != stamp) || unread {
                read_now.push(data);
            }
        }
    }
    if read_now.is_empty() {
        return false;
    }
    let states: BTreeMap<PathBuf, State> = read_now
        .into_iter()
        .filter_map(|data| Some((data.clone(), read(&data)?)))
        .collect();
    let mut saved = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut next = saved.clone();
    for (data, (stamp, _)) in &states {
        let covered = folders
            .iter()
            .filter(|(_, folder)| folder.parent() == Some(data.as_path()))
            .cloned()
            .collect();
        next.local_states.insert(data.clone(), (*stamp, covered));
    }
    for (id, folder) in folders {
        let (Some(data), Some(name)) = (folder.parent(), folder.file_name()) else {
            continue;
        };
        let Some((_, entries)) = states.get(data) else {
            continue;
        };
        let entry = entries.get(&*name.to_string_lossy());
        if let Some(record) = next.records.get_mut(&id) {
            // A folder missing from `Local State` keeps its last known name; `profile_presence` decides deletion.
            if let Some(entry) = entry {
                record.browser_name.clone_from(&entry.name);
                record.account.clone_from(&entry.account);
            }
        }
    }
    let changed = serde_json::to_vec(&next.records).ok() != serde_json::to_vec(&saved.records).ok();
    // Names are display facts: they are shown even when they cannot be saved, and read again after a restart.
    if changed && let Err(reason) = next.save(chrome.dir()) {
        tracing::warn!(%reason, "Browser profile names could not be saved");
    }
    *saved = next;
    changed
}
