//! The New chat screen's App preview website data becomes the session's that its first message started, so a
//! sign-in made in the preview before the chat is still there afterwards.
//!
//! Depends on: the preview service (`storage`, session locks), `ids` (`StorageKey`), the private settings write of
//! `service_pool`. Called by: vw-core-api `new_chat::adopt` (before the pages move), `storage` (every mount and
//! wipe), `session_ended`.
//! Invariants:
//! - Re-keyed, never copied or moved on disk: from the adoption on, the session's previews use the screen owner's
//!   storage key, which is the same data folder (Linux `WebKitGTK`, Windows `WebView2` user data folder and profile
//!   name) and the same `WKWebsiteDataStore` identifier (macOS). Renaming a store is not possible everywhere: macOS
//!   names an identified store by its UUID only, and on Linux a closed webview's web context keeps writing to its
//!   folder until the app quits.
//! - The record `<storage root>/adopted-storage.json` (session id -> screen owner id) is written before the
//!   adoption takes effect, so it holds after a restart; when it cannot be written nothing is adopted (the screen's
//!   data is then wiped as before and the session starts with its own).
//! - Only a screen owner id is ever adopted: the core makes a fresh one per draft and replaces it at the adoption
//!   and at Close all, so one key never serves two sessions and a wiped key is never used again. A session that
//!   already has website data keeps its own.
//! - The adopted data ends with the session: its delete wipes it, the screen owner's end never does
//!   (`session_ended`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::PreviewService;
use crate::engine::Storage;
use crate::ids::StorageKey;

/// The record's file name inside the storage root.
const FILE: &str = "adopted-storage.json";

/// Sessions using a New chat screen owner's website data.
pub(crate) struct Adoptions {
    /// Where the record is kept.
    file: PathBuf,
    /// Session id -> the screen owner id whose storage key the session's previews use.
    by_session: HashMap<String, String>,
}

impl Adoptions {
    /// The record under `root`; an unreadable or malformed record counts as empty, and an entry naming the session
    /// itself is ignored.
    pub(crate) fn load(root: &Path) -> Self {
        let file = root.join(FILE);
        let by_session: HashMap<String, String> = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            file,
            by_session: by_session
                .into_iter()
                .filter(|(session, owner)| {
                    !session.is_empty() && !owner.is_empty() && session != owner
                })
                .collect(),
        }
    }

    /// The owner whose storage key `session` uses (itself unless it adopted a screen's data).
    fn key_owner<'a>(&'a self, session: &'a str) -> &'a str {
        self.by_session.get(session).map_or(session, String::as_str)
    }

    /// Whether the store of `key` belongs to a session other than `owner` through an adoption.
    fn adopted_by_other(&self, owner: &str, key: StorageKey) -> bool {
        self.by_session
            .iter()
            .any(|(session, adopted)| session != owner && StorageKey::of(adopted) == key)
    }

    /// Replaces the record with `next`; the map changes only when the record was written.
    fn save(&mut self, next: HashMap<String, String>) -> Result<(), String> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
        super::service_pool::save_setting(&self.file, &data).map_err(|e| e.to_string())?;
        self.by_session = next;
        Ok(())
    }
}

impl PreviewService {
    fn adoptions(&self) -> std::sync::MutexGuard<'_, Adoptions> {
        self.adoptions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The storage key `owner`'s previews use: the screen owner's when `owner` adopted its data.
    pub(super) fn storage_key_of(&self, owner: &str) -> StorageKey {
        StorageKey::of(self.adoptions().key_owner(owner))
    }

    /// A mount uses its owner's own store `storage`: that owner has website data of its own from now on.
    pub(super) fn own_store(&self, storage: Storage) -> Storage {
        self.inner.lock().used_keys.insert(storage.key);
        storage
    }

    /// Whether `owner`'s end must leave the store of `key` alone: a session adopted it.
    pub(super) fn store_adopted_by_other(&self, owner: &str, key: StorageKey) -> bool {
        self.adoptions().adopted_by_other(owner, key)
    }

    /// Drops `session`'s adoption after its data was wiped (a failed record write only logs: the key is never
    /// reused, so a stale entry can only point at data that is gone).
    pub(super) fn forget_adoption(&self, session: &str) {
        let mut adoptions = self.adoptions();
        if !adoptions.by_session.contains_key(session) {
            return;
        }
        let mut next = adoptions.by_session.clone();
        next.remove(session);
        if let Err(error) = adoptions.save(next) {
            tracing::warn!(%error, "preview: the adopted data record was not updated");
        }
    }

    /// Whether `session` has website data of its own: it adopted a screen's before, a preview of it was mounted
    /// with its own store in this run, or its data folder exists (Linux and Windows, from an earlier run; on macOS
    /// a store from an earlier run is not looked up, which a session the first message just started never has).
    fn has_own_data(&self, session: &str) -> bool {
        if self.adoptions().by_session.contains_key(session) {
            return true;
        }
        let key = StorageKey::of(session);
        self.inner.lock().used_keys.contains(&key)
            || key.data_dir(&self.config.storage_root).exists()
    }

    /// The first message on the New chat screen started `session`: from now on `session`'s previews use the
    /// website data of the screen's owner `owner` (its App preview sign-ins and those of its own-server tabs).
    /// Call it before the pages move, so no page of `session` mounts its own store in between. Returns whether the
    /// data was adopted; `false` when `session` already has website data of its own (it keeps it; the caller wipes
    /// the screen's) or the record could not be written (the session starts with its own, as before).
    pub async fn adopt_storage(&self, owner: &str, session: &str) -> bool {
        if owner.is_empty() || session.is_empty() || owner == session {
            return false;
        }
        // The same order as a hand-off, so the two never wait on each other.
        let (first, second) = if owner <= session {
            (owner, session)
        } else {
            (session, owner)
        };
        let _first = self.session_lock(first).await;
        let _second = self.session_lock(second).await;
        // Not expected for a session the first message just started; its own data wins then.
        if self.has_own_data(session) {
            tracing::info!("preview: the started session has website data of its own; it keeps it");
            return false;
        }
        let mut adoptions = self.adoptions();
        let mut next = adoptions.by_session.clone();
        next.insert(session.to_owned(), owner.to_owned());
        match adoptions.save(next) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "preview: the draft's site data could not be kept for the session");
                false
            }
        }
    }
}
