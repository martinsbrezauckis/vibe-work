//! Session end, handoff and shutdown for every page owned by a Vibe session.
//! Depends on the preview service and native close seam; called by core lifecycle.

use std::path::{Path, PathBuf};

use super::PreviewService;
use crate::policy::{Destination, UrlPolicy};
use crate::state::Entry;

/// A moved page cannot inherit the previous session's port facts or project folder.
/// Core reopens local URLs after checking the new session's current port owner.
fn withdraw_inherited_rights(entry: &mut Entry, policy: &UrlPolicy) -> bool {
    let local = entry
        .url
        .as_ref()
        .is_some_and(|url| matches!(policy.classify(url), Ok(Destination::Loopback { .. })));
    entry.allowed.clear();
    entry.server_status = None;
    entry.downloads_dir = None;
    entry.downloads.clear();
    local
}

impl PreviewService {
    /// Rebinds every retained page's download destination to its new session folder after a handoff.
    /// The native mounts were closed before this call, so no old callback can acquire a new path.
    pub fn rebind_handoff_downloads(&self, owner: &str, dir: Option<&Path>) {
        let mut state = self.inner.lock();
        let mut pages = vec![owner.to_owned()];
        if let Some(tabs) = state.tabs.get(owner) {
            pages.extend(tabs.web.iter().cloned());
        }
        for key in pages {
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.downloads_dir = dir.map(Path::to_path_buf);
            }
        }
    }

    /// Restores only ports proven for the new owner of an unchanged local page after a handoff.
    /// A stale page key or URL cannot acquire the new owner's authority.
    pub async fn rebind_handoff_local_ports(
        &self,
        owner: &str,
        page_key: &str,
        url: &url::Url,
        allowed: &[u16],
    ) -> bool {
        let _activity = self.session_lock(page_key).await;
        {
            let mut state = self.inner.lock();
            if state.owner_of(page_key) != owner
                || !matches!(
                    self.inner.policy.classify(url),
                    Ok(Destination::Loopback { .. })
                )
            {
                return false;
            }
            let Some(entry) = state.entries.get_mut(page_key) else {
                return false;
            };
            if entry.url.as_ref() != Some(url) {
                return false;
            }
            entry.allowed = allowed.to_vec();
        }
        self.refresh_rules().await;
        true
    }

    /// A session ended (archive or delete): its preview closes; on delete its website data is wiped (never data a
    /// session adopted from this owner, `adopt_storage`) and the entry forgotten. Returns paths whose mount could not be closed or data could not be removed yet (the caller
    /// records them for a later retry).
    pub async fn session_ended(&self, session_id: &str, deleted: bool) -> Vec<PathBuf> {
        let web_keys = self.web_tabs(session_id);
        let mut leftovers = Vec::new();
        for key in &web_keys {
            let _page_activity = self.session_lock(key).await;
            if let Some(entry) = self.inner.lock().entries.get_mut(key) {
                entry.allowed.clear();
                entry.server_status = None;
            }
            if self.unmount(key).await.is_err() {
                leftovers.push(self.storage(key).dir);
            }
            self.forget_browser_turn(key);
        }
        let _activity = self.session_lock(session_id).await;
        // An ended session loses its local-port authority even when native close needs a retry.
        if let Some(entry) = self.inner.lock().entries.get_mut(session_id) {
            entry.allowed.clear();
            entry.server_status = None;
        }
        let app_closed = self.unmount(session_id).await.is_ok();
        if !app_closed {
            leftovers.push(self.storage(session_id).dir);
        }
        self.forget_browser_turn(session_id);
        // ended-session ports cannot remain in another preview's rules.
        self.refresh_rules().await;
        if !app_closed {
            return leftovers;
        }
        if !deleted {
            return leftovers;
        }
        // A failed native close retains the mount, its label, its owner and the retry identity.
        // Wiping website data while that view still exists would violate its storage boundary.
        if !leftovers.is_empty() {
            return leftovers;
        }
        {
            let mut state = self.inner.lock();
            for key in &web_keys {
                state.entries.remove(key);
                state.page_owners.remove(key);
                if state.is_shown(key) {
                    state.shown = None;
                }
            }
            state.entries.remove(session_id);
            state.tabs.remove(session_id);
        }
        let storage = self.storage(session_id);
        // A New chat screen owner whose data a session adopted: that data is the session's now, wiped only when
        // the session is deleted.
        if self.store_adopted_by_other(session_id, storage.key) {
            return leftovers;
        }
        match self.bounded("wipe", self.engine.wipe(&storage)).await {
            Ok(left) => leftovers.extend(left),
            Err(_) => leftovers.push(storage.dir),
        }
        self.forget_adoption(session_id);
        leftovers
    }

    /// Vibe moved session `old`'s conversation to `new` (`/clear`, a rewind into a new session, a
    /// compaction with a new id). The preview follows: its URL, title, width and console
    /// become `new`'s (and the window's shown preview, when it was `old`'s), so the Browser tab of the session the
    /// window shows keeps it. Its webview is closed: a session's website data lives in a folder named after the
    /// session id ([`crate::ids::StorageKey`]; after [`Self::adopt_storage`], after the New chat screen owner's id),
    /// so the next show or use mounts it again with `new`'s storage (the old session's data stays in its folder
    /// until that session is deleted). When `new` already has a
    /// preview of its own, `old`'s is dropped instead. Port rights and download paths are withdrawn before the
    /// core revalidates them for `new`. A failed native close leaves the old entry in place for retry.
    pub async fn session_handed_off(&self, old: &str, new: &str) {
        for key in self.web_tabs(old) {
            let _activity = self.session_lock(&key).await;
            if self.unmount(&key).await.is_err() {
                return;
            }
            self.forget_browser_turn(&key);
        }
        let _layout = self.layout_op.lock().await;
        let first = if old <= new { old } else { new };
        let second = if old <= new { new } else { old };
        let _first = self.session_lock(first).await;
        let _second = if first == second {
            None
        } else {
            Some(self.session_lock(second).await)
        };
        if self.unmount(old).await.is_err() {
            return;
        }
        self.forget_browser_turn(old);
        {
            let mut state = self.inner.lock();
            if let Some(mut entry) = state.entries.remove(old)
                && !state.entries.contains_key(new)
            {
                withdraw_inherited_rights(&mut entry, &self.inner.policy);
                new.clone_into(&mut entry.session_id);
                state.entries.insert(new.to_owned(), entry);
            }
            if let Some(shown) = state.shown.as_mut()
                && shown.session_id == old
            {
                new.clone_into(&mut shown.session_id);
            }
            if let Some(mut tabs) = state.tabs.remove(old) {
                if tabs.selected.as_deref() == Some(old) {
                    tabs.selected = Some(new.to_owned());
                }
                // Reading the destination's pinned App preview creates an empty tab list before promotion.
                // Only real destination web pages compete with the pages being handed over.
                if state.tabs.get(new).is_some_and(|tabs| !tabs.web.is_empty()) {
                    for key in tabs.web {
                        state.page_owners.remove(&key);
                        state.entries.remove(&key);
                    }
                } else {
                    for key in &tabs.web {
                        if let Some(entry) = state.entries.get_mut(key)
                            && withdraw_inherited_rights(entry, &self.inner.policy)
                        {
                            tabs.snapshots.remove(key);
                        }
                        state.page_owners.insert(key.clone(), new.to_owned());
                    }
                    state.tabs.insert(new.to_owned(), tabs);
                }
            }
        }
        self.inner.changed(old);
        self.inner.changed(new);
    }

    /// Closes every preview (quit). Website data stays.
    pub async fn shutdown(&self) {
        let _layout = self.layout_op.lock().await;
        let sessions: Vec<String> = self.inner.lock().entries.keys().cloned().collect();
        for session in sessions {
            let _activity = self.session_lock(&session).await;
            let _ = self.unmount(&session).await;
        }
    }
}
