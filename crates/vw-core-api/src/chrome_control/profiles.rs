//! Paired profile inventory and person-owned names/defaults. Authenticated helper status supplies identity;
//! Settings and chat selection read this store. Display names never authorize a route or prove a folder.
use super::{ChromeControl, agent_browser::Browser};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::PoisonError,
};

/// A paired browser profile, independent of the browser-wide availability switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfile {
    /// Immutable helper identity, including the browser namespace.
    pub profile_id: String,
    /// Browser namespace; adapters retain their actual profile capabilities.
    pub browser: String,
    /// Person-editable label, with a stable suffix when another profile has the same name.
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
    /// When `profile_presence` last searched for these unproven identities.
    #[serde(skip)]
    pub(super) scanned: Option<(std::time::Instant, BTreeSet<String>)>,
}
impl Profiles {
    pub(super) fn load(dir: Option<&Path>) -> Self {
        super::profile_storage::load(dir, "browser-profiles.json", 1024 * 1024).unwrap_or_else(
            |()| Self {
                storage_unavailable: true,
                ..Self::default()
            },
        )
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
    pub(super) fn name(&self, id: &str) -> String {
        let id = canonical(id);
        let base = self.base_name(&id);
        let browser = Browser::from_profile(&id);
        let duplicate = self.records.iter().any(|(other, record)| {
            other != &id
                && !record.removed
                && browser.owns_profile(other)
                && self.base_name(other) == base
        });
        if duplicate {
            format!("{base} · {}", short_id(&id))
        } else {
            base
        }
    }
    fn base_name(&self, id: &str) -> String {
        self.records
            .get(id)
            .and_then(|record| record.name.as_ref().or(record.metadata_name.as_ref()))
            .cloned()
            .unwrap_or_else(|| format!("Profile {}", short_id(id)))
    }
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
fn short_id(id: &str) -> String {
    blake3::hash(id.as_bytes()).to_hex()[..8].to_owned()
}
fn metadata_name(value: &Value) -> Option<String> {
    let name = value.as_str()?.trim();
    (!name.is_empty()
        && name.len() <= 160
        && !name.chars().any(char::is_control)
        && ![
            "Vibe Work",
            "Chrome",
            "Edge",
            "Google Chrome",
            "Microsoft Edge",
        ]
        .contains(&name))
    .then(|| name.to_owned())
}

pub(super) fn observe(chrome: &ChromeControl, data: &Value) {
    let Some(rows) = data["profiles"].as_array() else {
        return;
    };
    let mut profiles = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if profiles.storage_unavailable {
        return;
    }
    let before = serde_json::to_vec(&*profiles).ok();
    if let Some(owner) = data["ownerPublicKey"].as_str() {
        profiles.owner = Some(owner.into());
    }
    profiles.live.clear();
    for row in rows {
        let Some(id) = row["profileId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
        else {
            continue;
        };
        let id = canonical(id);
        if row["connected"] == true {
            profiles.live.insert(id.clone());
        }
        let owner = profiles.owner.clone();
        let record = profiles
            .records
            .entry(id.clone())
            .or_insert_with(|| Record {
                owner: owner.clone(),
                name: None,
                metadata_name: None,
                enabled: true,
                removed: false,
                folder: None,
                deleted: false,
            });
        let changed_owner = row["connected"] == true
            && record
                .owner
                .as_ref()
                .zip(owner.as_ref())
                .is_some_and(|(old, current)| old != current);
        if row["connected"] == true && owner.is_some() {
            record.owner = owner;
        }
        if row["connected"] == true {
            // A live authenticated connection outranks any earlier deletion evidence.
            record.deleted = false;
        }
        if let Some(name) = metadata_name(&row["name"]) {
            record.metadata_name = Some(name);
        }
        if changed_owner {
            profiles.defaults.retain(|_, selected| selected != &id);
        }
    }
    if before != serde_json::to_vec(&*profiles).ok()
        && let Err(reason) = profiles.save(chrome.dir())
    {
        tracing::warn!(%reason, "Browser profile inventory could not be saved");
    }
}

pub(super) fn view(shared: &crate::core::Shared) -> Vec<BrowserProfile> {
    let chrome = &shared.chrome_control;
    let profiles = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    profiles
        .records
        .iter()
        .filter(|(_, record)| !record.removed)
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
            })
        })
        .collect()
}

pub(super) fn label(chrome: &ChromeControl, id: &str) -> String {
    let name = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .name(id);
    format!("{} · {name}", Browser::from_profile(id).short_name())
}

pub(super) fn check(chrome: &ChromeControl, id: &str) -> Result<(), String> {
    {
        let saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
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
        "{} is unavailable; no action ran. Select a connected profile or reconnect this profile.",
        label(chrome, id)
    ))
}

/// The browser deleted this identity's profile (positive evidence only); a bound chat names it.
pub(super) fn deleted(chrome: &ChromeControl, id: &str) -> bool {
    chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .records
        .get(&canonical(id))
        .is_some_and(|record| record.deleted)
}

pub(super) fn optional_view(shared: &crate::core::Shared) -> Option<Vec<BrowserProfile>> {
    let rows = view(shared);
    (!rows.is_empty()).then_some(rows)
}

pub(super) fn disconnected(chrome: &ChromeControl) {
    chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .live
        .clear();
}

/// A successful app-only exact-tab read proves this paired identity is live; it supplies no folder or name.
pub(super) fn observed_tab(chrome: &ChromeControl, id: &str) {
    let mut saved = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if saved.storage_unavailable {
        return;
    }
    let id = canonical(id);
    let owner = saved.owner.clone();
    saved.records.entry(id.clone()).or_insert(Record {
        owner,
        name: None,
        metadata_name: None,
        enabled: true,
        removed: false,
        folder: None,
        deleted: false,
    });
    saved.live.insert(id);
}

/// A removed pin can request a fresh native ceremony; an identity switched Off cannot.
pub(super) fn pairing_allowed(chrome: &ChromeControl, id: &str) -> bool {
    let saved = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    !saved.storage_unavailable
        && saved
            .records
            .get(&canonical(id))
            .is_none_or(|record| record.enabled || record.removed)
}

/// Called only after the person approves the native pairing dialog, before accepting the new signed key.
pub(super) fn reapprove(chrome: &ChromeControl, id: &str) -> Result<(), String> {
    let mut saved = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
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
