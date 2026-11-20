//! Paired profile inventory and person-owned defaults. Authenticated helper status supplies identity;
//! Settings and chat selection read this store. Display names never authorize a route or prove a folder.
//! A profile is named as the browser's own profile picker names it (`profile_names`), else by browser and pairing
//! order ("Chrome profile 2"); a name typed in an earlier version is kept only for an identity the browser cannot name.
//! Agent text adds the signed-in account's domain to that name ("Alex (company.example)"), never the account itself.
use super::{ChromeControl, agent_browser::Browser};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use vw_platform::sync::LockRecover;
#[path = "profiles_observe.rs"]
mod observation;
pub(super) use observation::observe;
#[path = "profiles_view.rs"]
mod views;
pub(super) use views::{agent_inventory, agent_view, optional_view, view};
#[path = "profiles_removal.rs"]
mod removal;
pub(super) use removal::{
    REMOVAL_NOT_SAVED, all_removed, forget_all, pairing_allowed, reapprove, remove_browser,
    remove_browser_saved, removed_not_connected, retired, retired_by,
};

mod names;

/// A paired browser profile, independent of the browser-wide availability switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfile {
    /// Immutable helper identity, including the browser namespace.
    pub profile_id: String,
    /// Browser namespace; adapters retain their actual profile capabilities.
    pub browser: String,
    /// The browser's own profile name (or "Chrome profile 2" when it cannot be read), with the pairing number
    /// added when another profile of the same browser has the same name.
    pub name: String,
    /// Short current connection state.
    pub state: ProfileState,
    /// This identity's own availability switch.
    pub enabled: bool,
    /// Connected and available for a new binding.
    pub eligible: bool,
    /// The person's default for new chats using this browser.
    pub is_default: bool,
    /// Short reason for an unavailable retained identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub reason: Option<String>,
    /// The signed-in account the browser shows under the profile name. Screen only: agent text carries only its domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub account: Option<String>,
}

/// Connection evidence for one authenticated profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ProfileState {
    /// A live authenticated connection.
    Connected,
    /// Paired before, with no current connection; absence is not deletion.
    Offline,
    /// Positive evidence tied to this identity proves the extension is missing.
    ExtensionMissing,
    /// Off, removed, or unavailable with a specific reason.
    Unavailable,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) owner: Option<String>,
    pub(super) name: Option<String>,
    pub(super) metadata_name: Option<String>,
    pub(super) enabled: bool,
    pub(super) removed: bool,
    /// The profile folder whose extension storage holds this identity's UUID (`profile_presence`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) folder: Option<std::path::PathBuf>,
    /// Positive evidence: the proven folder was deleted by the browser. Cleared by a live connection.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) deleted: bool,
    /// The browser's own name for the proven folder (`profile_names`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) browser_name: Option<String>,
    /// The signed-in account shown under that name; screen only, agent text carries only its domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) account: Option<String>,
    /// Pairing order within its browser, from 1; never reused, so a fallback name stays stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) number: Option<u32>,
    /// When this identity was first seen or last approved in a pairing, in milliseconds since the Unix epoch; tells
    /// same-named profiles apart for the person. Absent for identities saved before this was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) paired_at: Option<u64>,
    /// A later pairing of the same browser the person approved replaced this identity while it was not connected
    /// (Safari and Firefox, whose profiles the browser cannot name). Hidden from Settings and the profile card; its
    /// record and pin are kept, and a live connection clears it. Holds the identity that replaced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) replaced_by: Option<String>,
}
impl Record {
    fn new(owner: Option<String>, number: u32) -> Self {
        Self {
            owner,
            name: None,
            metadata_name: None,
            enabled: true,
            removed: false,
            folder: None,
            deleted: false,
            browser_name: None,
            account: None,
            number: Some(number),
            paired_at: Some(now_ms()),
            replaced_by: None,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Profiles {
    #[serde(skip)]
    storage_unavailable: bool,
    pub(super) records: BTreeMap<String, Record>,
    pub(super) defaults: BTreeMap<String, String>,
    #[serde(skip)]
    live: BTreeSet<String>,
    #[serde(skip)]
    pub(super) owner: Option<String>,
    #[serde(skip)]
    pub(super) generations: BTreeMap<String, u64>,
    /// When `profile_presence` last searched for these unproven identities, and how many searches for this same
    /// set came before it.
    #[serde(skip)]
    pub(super) scanned: Option<(std::time::Instant, BTreeSet<String>, usize)>,
    /// The modification time and size of each `Local State` last read by `profile_names`, with the identities and
    /// proven folders that read covered: a newer identity proven in a folder read before is not covered.
    #[serde(skip)]
    pub(super) local_states: BTreeMap<std::path::PathBuf, super::profile_names::Read>,
}
impl Profiles {
    pub(super) fn load(dir: Option<&Path>) -> Self {
        let mut loaded: Self =
            super::profile_storage::load(dir, "browser-profiles.json", 1024 * 1024).unwrap_or_else(
                |()| Self {
                    storage_unavailable: true,
                    ..Self::default()
                },
            );
        loaded.number_unnumbered();
        loaded
    }
    /// Records saved before pairing numbers existed get the next numbers of their browser, in identity order.
    fn number_unnumbered(&mut self) {
        let unnumbered: Vec<String> = self
            .records
            .iter()
            .filter(|(_, record)| record.number.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        for id in unnumbered {
            let number = self.next_number(&id);
            if let Some(record) = self.records.get_mut(&id) {
                record.number = Some(number);
            }
        }
    }
    /// The next pairing number for a new identity of `id`'s browser.
    fn next_number(&self, id: &str) -> u32 {
        let browser = Browser::from_profile(id);
        self.records
            .iter()
            .filter(|(other, _)| browser.owns_profile(other))
            .filter_map(|(_, record)| record.number)
            .max()
            .unwrap_or_default()
            .saturating_add(1)
    }
    /// The signed-in account the browser shows for this identity; screen only.
    pub(super) fn account(&self, id: &str) -> Option<String> {
        self.records.get(&canonical(id))?.account.clone()
    }
    pub(super) fn save(&self, dir: Option<&Path>) -> Result<(), String> {
        if self.storage_unavailable {
            return Err(super::profile_storage::UNREADABLE.into());
        }
        let Some(dir) = dir else { return Ok(()) };
        let failed = || "The browser profile setting could not be saved. Try again.".to_owned();
        std::fs::create_dir_all(dir).map_err(|_| failed())?;
        let bytes = serde_json::to_vec(self).map_err(|_| failed())?;
        if bytes.len() > 1024 * 1024 {
            return Err(failed());
        }
        vw_platform::replace_private(&dir.join("browser-profiles.json"), &bytes)
            .map_err(|_| failed())
    }
    pub(super) fn enabled(&self, id: &str) -> bool {
        !self.storage_unavailable
            && self
                .records
                .get(&canonical(id))
                .is_none_or(|record| record.enabled && !record.removed && !record.deleted)
    }
    pub(super) fn is_live(&self, id: &str) -> bool {
        self.live.contains(id)
    }
    pub(super) fn generation(&self, id: &str) -> u64 {
        self.generations
            .get(&canonical(id))
            .copied()
            .unwrap_or_default()
    }
    pub(super) fn disabled(&self) -> Vec<String> {
        self.records
            .iter()
            .filter(|(_, record)| !record.enabled && !record.removed)
            .map(|(id, _)| id.clone())
            .collect()
    }
    /// Whether this identity is named by browser and pairing order alone.
    fn numbered_only(&self, id: &str) -> bool {
        self.records.get(id).is_none_or(|record| {
            record.browser_name.is_none()
                && record.name.is_none()
                && usable_metadata(record).is_none()
        })
    }
    /// Listed in Settings and on the profile card: paired, not removed, not retired (`retired`), and not the owned
    /// agent Firefox window, which is no person's profile.
    pub(super) fn shown(&self, id: &str) -> bool {
        id != AGENT_FIREFOX
            && self.records.get(id).is_some_and(|record| !record.removed)
            && !self.retired(id)
    }
    /// A newer identity of the same browser profile replaced this one, and it is not connected now: a later pairing
    /// superseded it (`paired`), or a newer Chrome or Edge identity was proven in the same profile folder (one folder
    /// keeps one identity in its extension storage). Removing that newer identity leaves the older one retired: the
    /// folder still holds the newer identity, so the older one cannot connect from it again. A live connection always
    /// shows it again.
    pub(super) fn retired(&self, id: &str) -> bool {
        let Some(record) = self.records.get(id) else {
            return false;
        };
        if self.live.contains(id) {
            return false;
        }
        if record.replaced_by.is_some() {
            return true;
        }
        let Some(folder) = record.folder.as_ref() else {
            return false;
        };
        let browser = Browser::from_profile(id);
        self.records.iter().any(|(other, newer)| {
            other != id
                && browser.owns_profile(other)
                && newer.replaced_by.is_none()
                && newer.folder.as_ref() == Some(folder)
                && (newer.number, other.as_str()) > (record.number, id)
        })
    }
}

/// The helper's owned agent Firefox window. It reports like a profile, but it belongs to Vibe Work.
pub(super) const AGENT_FIREFOX: &str = "firefox:agent";

/// Names a browser or extension registers under, never a profile's own name: the helper replaces every
/// registration name with the browser's product name.
const REGISTRATION_NAMES: [&str; 11] = [
    "Vibe Work",
    "Chrome",
    "Edge",
    "Google Chrome",
    "Microsoft Edge",
    "Safari",
    "Firefox",
    "Mozilla Firefox",
    "Your Safari",
    "Your Firefox",
    "Your Chrome",
];

/// A saved helper-reported name that is a real profile name, not a registration name saved by an earlier version.
fn usable_metadata(record: &Record) -> Option<&String> {
    record
        .metadata_name
        .as_ref()
        .filter(|name| !REGISTRATION_NAMES.contains(&name.trim()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|since| u64::try_from(since.as_millis()).ok())
        .unwrap_or_default()
}

/// "paired 12 Nov 23:25" in local time (the year when it is not this year); "paired earlier" without a time.
fn paired_label(at: Option<u64>, seconds: bool) -> String {
    crate::date_text::local_profile_date(at, seconds)
        .map_or_else(|| "paired earlier".into(), |date| format!("paired {date}"))
}

pub(super) fn canonical(id: &str) -> String {
    if super::enroll::uuid_shape(id) {
        format!("chrome:{}", id.to_ascii_lowercase())
    } else if let Some((browser, uuid)) = id.split_once(':')
        && super::enroll::uuid_shape(uuid)
    {
        format!("{browser}:{}", uuid.to_ascii_lowercase())
    } else {
        id.to_owned()
    }
}
fn metadata_name(value: &Value) -> Option<String> {
    let name = value.as_str()?.trim();
    (!name.is_empty()
        && name.len() <= 160
        && !name.chars().any(char::is_control)
        && !REGISTRATION_NAMES.contains(&name))
    .then(|| name.to_owned())
}

/// The profile's name for agent and status text: "Chrome · Work (company.example)", or "Chrome profile 2" when the name
/// is only the pairing order (`Profiles::agent_name`). Carries the account's domain, never the account.
pub(super) fn label(chrome: &ChromeControl, id: &str) -> String {
    let saved = chrome.profiles.lock_or_recover();
    let name = saved.agent_name(id);
    if saved.numbered_only(&canonical(id)) {
        name
    } else {
        format!("{} · {name}", Browser::from_profile(id).short_name())
    }
}

pub(super) fn check(chrome: &ChromeControl, id: &str) -> Result<(), String> {
    {
        let saved = chrome.profiles.lock_or_recover();
        if saved.storage_unavailable {
            return Err(super::profile_storage::UNREADABLE.into());
        }
        if saved.enabled(id) {
            return Ok(());
        }
        if saved
            .records
            .get(&canonical(id))
            .is_some_and(|record| record.deleted && !record.removed)
        {
            drop(saved);
            return Err(super::profile_presence::removed_message(chrome, id));
        }
    }
    Err(format!(
        "{} is unavailable; no action ran. Use another connected profile, or ask the user to reconnect this profile in Settings > Browser.",
        label(chrome, id)
    ))
}

/// The browser deleted this identity's profile (positive evidence only); a bound chat names it.
pub(super) fn deleted(chrome: &ChromeControl, id: &str) -> bool {
    chrome
        .profiles
        .lock_or_recover()
        .records
        .get(&canonical(id))
        .is_some_and(|record| record.deleted)
}

pub(super) fn disconnected(chrome: &ChromeControl) {
    chrome.profiles.lock_or_recover().live.clear();
}

/// A successful app-only exact-tab read proves this paired identity is live; it supplies no folder or name.
pub(super) fn observed_tab(chrome: &ChromeControl, id: &str) {
    let mut saved = chrome.profiles.lock_or_recover();
    if saved.storage_unavailable {
        return;
    }
    let id = canonical(id);
    let owner = saved.owner.clone();
    let number = saved.next_number(&id);
    saved
        .records
        .entry(id.clone())
        .or_insert_with(|| Record::new(owner, number));
    saved.live.insert(id);
}

/// Called after the person approved a pairing and the helper saved its pin. Stamps the pairing time. For Safari and
/// Firefox, every other identity of that browser that is not connected now is superseded: the extension keeps one
/// identity per profile in its own storage, and the browser names no profile, so a new pairing while the old
/// identity is silent is the same browser profile starting over (a reinstalled or updated extension). The old record
/// and pin stay; a live connection of the old identity proves a second profile and shows it again. Identities
/// connected at this moment are never superseded. `found` is the Chrome or Edge folder the pairing dialog proved for
/// this identity with its `Local State` entry: an identity without a proven folder gets it and the browser's name at
/// once, so a profile paired again after its extension was installed again is named as before, and the older identity
/// of that folder is retired (`Profiles::retired`), without waiting for the next folder search.
pub(super) fn paired(
    chrome: &ChromeControl,
    id: &str,
    found: Option<super::profile_presence::Found>,
) {
    let mut saved = chrome.profiles.lock_or_recover();
    if saved.storage_unavailable {
        return;
    }
    let mut next = saved.clone();
    let id = canonical(id);
    let owner = next.owner.clone();
    let number = next.next_number(&id);
    let record = next
        .records
        .entry(id.clone())
        .or_insert_with(|| Record::new(owner, number));
    record.paired_at = Some(now_ms());
    record.replaced_by = None;
    if let Some((folder, entry)) = found
        && record.folder.is_none()
        && !record.deleted
    {
        record.folder = Some(folder);
        record.browser_name = entry.name;
        record.account = entry.account;
    }
    let browser = Browser::from_profile(&id);
    if matches!(browser, Browser::Safari | Browser::Firefox) && id != AGENT_FIREFOX {
        let live = next.live.clone();
        for (other, record) in &mut next.records {
            if other != &id
                && other != AGENT_FIREFOX
                && browser.owns_profile(other)
                && !record.removed
                && !live.contains(other)
            {
                record.replaced_by = Some(id.clone());
            }
        }
    }
    if let Err(reason) = next.save(chrome.dir()) {
        tracing::warn!(%reason, "Browser profile inventory could not be saved");
        return;
    }
    *saved = next;
}
