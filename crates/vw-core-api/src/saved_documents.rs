//! Chat-owned saved attachment grants and successful document text, held only in core memory.
//! Used by documents/save, removal, session deletion and `document_read`; paths require the saved SHA-256.
//! A per-chat read lock shares concurrent extraction; full caches refuse new work without evicting results.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use sha2::{Digest as _, Sha256};

use crate::core::Shared;
use crate::documents_view::KeptView;

const CHAT_GONE: &str = "This chat is no longer available.";
const CACHE_FULL: &str = "This chat has reached its document reading limit.";
const MAX_RESULTS: usize = 32;
const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
// Reserve the largest bounded extraction before spending OCR, including four-byte UTF-8 characters.
const RESULT_BUDGET: usize = vw_documents::MAX_TEXT_CHARS * 4;

pub(crate) type Digest = [u8; 32];

pub(crate) fn digest(bytes: &[u8]) -> Digest {
    Sha256::digest(bytes).into()
}

fn locked<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub(crate) struct Text {
    pub(crate) text: String,
    pub(crate) kept: Option<KeptView>,
}

struct Attachment {
    id: String,
    digest: Digest,
}

#[derive(Default)]
struct State {
    revoked: bool,
    paths: BTreeMap<PathBuf, Attachment>,
    sources: BTreeMap<PathBuf, PathBuf>,
    text: BTreeMap<Digest, Text>,
    bytes: usize,
    reserved: bool,
}

pub(crate) type ReadReceiver = tokio::sync::watch::Receiver<Option<Result<Text, String>>>;

#[derive(Default)]
pub(crate) struct Chat {
    pub(crate) reading: tokio::sync::Mutex<()>,
    pub(crate) jobs: Mutex<BTreeMap<Digest, ReadReceiver>>,
    state: Mutex<State>,
}

impl State {
    fn authorizes(&self, path: &Path, digest: Digest) -> Result<(), String> {
        if self.revoked {
            return Err(CHAT_GONE.to_owned());
        }
        match self.paths.get(path) {
            Some(attachment) if attachment.digest == digest => Ok(()),
            Some(_) => Err(format!(
                "{} has changed since it was attached.",
                path.display()
            )),
            None => Err(format!(
                "{} is not a saved attachment in this chat.",
                path.display()
            )),
        }
    }

    fn room(&self) -> bool {
        let reserved = usize::from(self.reserved);
        self.text.len() + reserved < MAX_RESULTS
            && self.bytes + RESULT_BUDGET * (1 + reserved) <= MAX_TEXT_BYTES
    }

    fn result_fits(&self) -> bool {
        self.text.len() < MAX_RESULTS && self.bytes + RESULT_BUDGET <= MAX_TEXT_BYTES
    }
}

pub(crate) struct Reservation<'a>(&'a Chat);

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        locked(&self.0.state).reserved = false;
    }
}

impl Chat {
    /// An explicitly allowed project file is now an attachment; repeated reads reuse that exact saved copy.
    pub(crate) fn source(&self, path: &Path, digest: Digest) -> Option<PathBuf> {
        let state = locked(&self.state);
        let saved = state.sources.get(path)?;
        state.authorizes(saved, digest).ok()?;
        Some(saved.clone())
    }

    pub(crate) fn link_source(
        &self,
        source: PathBuf,
        saved: PathBuf,
        digest: Digest,
    ) -> Result<(), String> {
        let mut state = locked(&self.state);
        state.authorizes(&saved, digest)?;
        state.sources.insert(source, saved);
        Ok(())
    }
    /// Keeps space for the result while the composer may finish another explicit read.
    pub(crate) fn reserve(&self) -> Result<Reservation<'_>, String> {
        let mut state = locked(&self.state);
        if state.revoked {
            return Err(CHAT_GONE.to_owned());
        }
        if !state.room() {
            return Err(CACHE_FULL.to_owned());
        }
        state.reserved = true;
        Ok(Reservation(self))
    }

    pub(crate) fn expected(&self, path: &Path) -> Result<Digest, String> {
        let state = locked(&self.state);
        if state.revoked {
            return Err(CHAT_GONE.to_owned());
        }
        state
            .paths
            .get(path)
            .map(|a| a.digest)
            .ok_or_else(|| format!("{} is not a saved attachment in this chat.", path.display()))
    }

    /// Checks authorization even on a cache hit. Call while holding `reading` through completion.
    pub(crate) fn cached(&self, path: &Path, digest: Digest) -> Result<Option<Text>, String> {
        let state = locked(&self.state);
        state.authorizes(path, digest)?;
        if let Some(text) = state.text.get(&digest) {
            return Ok(Some(text.clone()));
        }
        if !state.room() {
            return Err(CACHE_FULL.to_owned());
        }
        Ok(None)
    }

    /// A removed attachment or deleted chat cannot regain its grant when an in-flight read finishes.
    pub(crate) fn remember(&self, path: &Path, digest: Digest, text: Text) -> Result<(), String> {
        let mut state = locked(&self.state);
        state.authorizes(path, digest)?;
        if !state.text.contains_key(&digest) {
            if !state.result_fits() || state.bytes + text.text.len() > MAX_TEXT_BYTES {
                return Err(CACHE_FULL.to_owned());
            }
            state.bytes += text.text.len();
            state.text.insert(digest, text);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct SavedDocuments(Mutex<BTreeMap<String, Arc<Chat>>>);

impl SavedDocuments {
    /// Registers only bytes saved from the pending store. Session presence and registration are ordered
    /// with `forget`, which deletion calls immediately after removing the session from the store.
    pub(crate) fn register(
        &self,
        shared: &Shared,
        session: &str,
        id: &str,
        path: PathBuf,
        digest: Digest,
    ) -> Result<(), String> {
        let mut chats = locked(&self.0);
        if shared.store.view(session).is_none() {
            return Err(CHAT_GONE.to_owned());
        }
        let chat = chats.entry(session.to_owned()).or_default();
        locked(&chat.state).paths.insert(
            path,
            Attachment {
                id: id.to_owned(),
                digest,
            },
        );
        Ok(())
    }

    pub(crate) fn chat(&self, session: &str) -> Result<Arc<Chat>, String> {
        locked(&self.0)
            .get(session)
            .cloned()
            .ok_or_else(|| "This chat has no saved attachments.".to_owned())
    }

    /// Reuses text the composer has already read; automatic attachment reading keeps its own lifecycle.
    pub(crate) fn seed(&self, id: &str, text: &Text) {
        for chat in locked(&self.0).values() {
            let mut state = locked(&chat.state);
            let digest = state.paths.values().find(|a| a.id == id).map(|a| a.digest);
            if let Some(digest) = digest
                && !state.revoked
                && !state.text.contains_key(&digest)
                && state.room()
                && state.bytes + text.text.len() <= MAX_TEXT_BYTES
            {
                state.bytes += text.text.len();
                state.text.insert(digest, text.clone());
            }
        }
    }

    pub(crate) fn remove(&self, id: &str) -> bool {
        let mut removed = false;
        for chat in locked(&self.0).values() {
            let mut state = locked(&chat.state);
            let before = state.paths.len();
            state.paths.retain(|_, attachment| attachment.id != id);
            removed |= before != state.paths.len();
        }
        removed
    }

    pub(crate) fn forget(&self, session: &str) {
        if let Some(chat) = locked(&self.0).remove(session) {
            *locked(&chat.state) = State {
                revoked: true,
                ..State::default()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: usize) -> Text {
        Text {
            text: "x".repeat(bytes),
            kept: None,
        }
    }

    fn grant(chat: &Chat, n: u8) -> (PathBuf, Digest) {
        let path = PathBuf::from(format!("attachments/chat/{n}.txt"));
        let digest = [n; 32];
        locked(&chat.state).paths.insert(
            path.clone(),
            Attachment {
                id: n.to_string(),
                digest,
            },
        );
        (path, digest)
    }

    #[test]
    fn full_count_and_byte_budgets_keep_successes_and_refuse_new_work() {
        for size in [1, RESULT_BUDGET] {
            let chat = Chat::default();
            let mut saved = Vec::new();
            for n in 0..=u8::try_from(MAX_RESULTS).unwrap() {
                let (path, digest) = grant(&chat, n);
                if chat.cached(&path, digest).is_err() {
                    assert_ne!(saved.len(), 0);
                    assert!(chat.remember(&path, digest, text(size)).is_err());
                    break;
                }
                chat.remember(&path, digest, text(size)).unwrap();
                saved.push((path, digest));
            }
            assert_eq!(
                saved.len(),
                if size == 1 {
                    MAX_RESULTS
                } else {
                    MAX_TEXT_BYTES / RESULT_BUDGET
                }
            );
            for (path, digest) in saved {
                assert_eq!(
                    chat.cached(&path, digest).unwrap().unwrap().text.len(),
                    size
                );
            }
            assert!(locked(&chat.state).bytes <= MAX_TEXT_BYTES);
        }
    }

    #[test]
    fn reserved_result_keeps_its_capacity_until_completion_or_failure() {
        let chat = Chat::default();
        for n in 0..u8::try_from(MAX_RESULTS).unwrap() - 1 {
            let (path, digest) = grant(&chat, n);
            chat.remember(&path, digest, text(1)).unwrap();
        }
        let (path, digest) = grant(&chat, u8::try_from(MAX_RESULTS).unwrap());
        {
            let _reservation = chat.reserve().unwrap();
            assert!(!locked(&chat.state).room());
            assert!(locked(&chat.state).result_fits());
        }
        assert!(locked(&chat.state).room());
        let _reservation = chat.reserve().unwrap();
        chat.remember(&path, digest, text(1)).unwrap();
        assert!(chat.cached(&path, digest).unwrap().is_some());
    }

    #[test]
    fn deletion_clears_cache_and_revokes_in_flight_read_handles() {
        let registry = SavedDocuments::default();
        let chat = Arc::new(Chat::default());
        let (path, digest) = grant(&chat, 1);
        chat.remember(&path, digest, text(3)).unwrap();
        locked(&registry.0).insert("chat".to_owned(), chat.clone());
        registry.forget("chat");
        assert!(registry.chat("chat").is_err());
        assert!(chat.cached(&path, digest).is_err());
        assert!(chat.remember(&path, digest, text(3)).is_err());
        assert!(locked(&chat.state).text.is_empty());
        assert_eq!(locked(&chat.state).bytes, 0);
    }
}
