//! Reading the helper's reported profile rows into the remembered profile records.
use super::{AGENT_FIREFOX, ChromeControl, LockRecover, Record, Value, canonical, metadata_name};

pub(in crate::chrome_control) fn observe(chrome: &ChromeControl, data: &Value) {
    let Some(rows) = data["profiles"].as_array() else {
        return;
    };
    let mut profiles = chrome.profiles.lock_or_recover();
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
        if id == AGENT_FIREFOX {
            continue;
        }
        if row["connected"] == true {
            profiles.live.insert(id.clone());
        }
        let owner = profiles.owner.clone();
        let number = profiles.next_number(&id);
        let record = profiles
            .records
            .entry(id.clone())
            .or_insert_with(|| Record::new(owner.clone(), number));
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
            // A live authenticated connection outranks any earlier deletion evidence, and proves a superseded
            // identity is a different running profile.
            record.deleted = false;
            record.replaced_by = None;
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
