//! What the core remembers about sessions it has seen in listings, for three trusted-side uses:
//! naming a session in the native delete dialog, knowing which folders sessions work in (so
//! "reveal folder" can refuse a path no session uses), and the folder a session's terminals start
//! in (from the core's record, never from a client).
//!
//! Depends on: `serde_json`, vw-session (`SessionStore`, live titles and folders of loaded
//! sessions). Called by: `dispatch` (records each listing page), `confirmer` (dialog text),
//! `core` (`Core::is_session_folder`).
//! Invariants:
//! - Text from Vibe's store is bounded. Display names collapse whitespace; native confirmations
//!   retain control characters for the dialog formatter to visibly escape.
//! - Bounded: past `MAX_ENTRIES` the oldest-inserted half of the titles is forgotten; the dialog
//!   then uses the untitled-session label, never a wrong title.
//! - The lock is never held across an await (plain methods, no futures).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;
use vw_session::SessionStore;

const MAX_ENTRIES: usize = 20_000;
const TITLE_MAX_CHARS: usize = 80;

#[derive(Clone)]
struct Title {
    display: String,
    confirmation: String,
}

impl Title {
    fn new(text: &str) -> Self {
        Self {
            display: clean(text),
            confirmation: shorten(text),
        }
    }
}

#[derive(Default)]
struct Inner {
    titles: HashMap<String, Title>,
    order: VecDeque<String>,
    folders: HashMap<String, BTreeSet<String>>,
    /// The listed `cwd` of each session (bounded like `titles`).
    cwds: HashMap<String, String>,
    /// Session ids never repeat. Ignore a listing already in flight when deletion finished.
    deleted: BTreeSet<String>,
}

/// Titles and folders seen in `session/list` pages.
#[derive(Default)]
pub(crate) struct Names {
    inner: Mutex<Inner>,
}

impl Names {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records the title and folders of every item of a listing page.
    pub(crate) fn note_listing(&self, items: &[Value]) {
        let mut inner = self.lock();
        for item in items {
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                if inner.deleted.contains(id) {
                    continue;
                }
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|t| !t.trim().is_empty())
                    .or_else(|| item.get("preview").and_then(Value::as_str))
                    .map(Title::new);
                if let Some(title) = title.filter(|t| !t.display.is_empty())
                    && inner.titles.insert(id.to_owned(), title).is_none()
                {
                    inner.order.push_back(id.to_owned());
                }
                if let Some(cwd) = item.get("cwd").and_then(Value::as_str) {
                    inner.cwds.insert(id.to_owned(), cwd.to_owned());
                }
                let mut folders = BTreeSet::new();
                note_folders(&mut folders, item);
                inner.folders.insert(id.to_owned(), folders);
            }
        }
        while inner.titles.len() > MAX_ENTRIES {
            for _ in 0..MAX_ENTRIES / 2 {
                if let Some(old) = inner.order.pop_front() {
                    inner.titles.remove(&old);
                }
            }
        }
        if inner.cwds.len() > MAX_ENTRIES {
            // Forgotten folders are found again in the store or the next listing.
            inner.cwds.clear();
        }
        if inner.folders.len() > MAX_ENTRIES {
            inner.folders.clear();
        }
    }

    /// A deleted chat grants no folder access, including through an earlier listing.
    pub(crate) fn forget(&self, id: &str) {
        let mut inner = self.lock();
        inner.titles.remove(id);
        inner.order.retain(|entry| entry != id);
        inner.cwds.remove(id);
        inner.folders.remove(id);
        inner.deleted.insert(id.to_owned());
    }

    /// The folder session `id` works in: the loaded session's `cwd`, else the listed one.
    pub(crate) fn cwd_of(&self, store: &SessionStore, id: &str) -> Option<String> {
        let live = store.view(id).and_then(|view| {
            view.state
                .pointer("/session/cwd")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        live.or_else(|| self.lock().cwds.get(id).cloned())
            .filter(|cwd| !cwd.is_empty())
    }

    /// A cleaned title for `id`: the live title of a loaded session, else the listed one.
    pub(crate) fn title_of(&self, store: &SessionStore, id: &str) -> Option<String> {
        self.title(store, id).map(|title| title.display)
    }

    /// A bounded title whose controls the native dialog formatter must visibly escape.
    pub(crate) fn confirmation_title(&self, store: &SessionStore, id: &str) -> String {
        self.title(store, id).map_or_else(
            || {
                vw_vocabulary::text("untitledSession")
                    .unwrap_or_default()
                    .to_owned()
            },
            |title| title.confirmation,
        )
    }

    fn title(&self, store: &SessionStore, id: &str) -> Option<Title> {
        let live = store.view(id).and_then(|view| {
            let session = view.state.get("session")?;
            session
                .get("title")
                .and_then(Value::as_str)
                .filter(|t| !t.trim().is_empty())
                .or_else(|| session.get("preview").and_then(Value::as_str))
                .map(Title::new)
        });
        live.filter(|t| !t.display.is_empty())
            .or_else(|| self.lock().titles.get(id).cloned())
    }

    /// The session's visible name: title, first-message preview, then the untitled-session label.
    pub(crate) fn display_title(&self, store: &SessionStore, id: &str) -> String {
        self.title_of(store, id).unwrap_or_else(|| {
            vw_vocabulary::text("untitledSession")
                .unwrap_or_default()
                .to_owned()
        })
    }

    /// Every `(session id, cwd)` known: loaded sessions' live folders, then listed ones (the folders a
    /// file search covers).
    pub(crate) fn session_folders(&self, store: &SessionStore) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = store
            .views()
            .into_iter()
            .filter_map(|view| {
                let cwd = view.state.pointer("/session/cwd").and_then(Value::as_str)?;
                Some((view.session_id, cwd.to_owned()))
            })
            .collect();
        let loaded: BTreeSet<String> = out.iter().map(|(id, _)| id.clone()).collect();
        out.extend(
            self.lock()
                .cwds
                .iter()
                .filter(|(id, cwd)| !loaded.contains(*id) && !cwd.is_empty())
                .map(|(id, cwd)| (id.clone(), cwd.clone())),
        );
        out
    }

    /// True when a listed or loaded session works in `path` (or has it as a workspace root).
    pub(crate) fn is_session_folder(&self, store: &SessionStore, path: &str) -> bool {
        self.folders(store).contains(path)
    }

    /// Current session roots, with loaded state replacing any older listing for that session.
    pub(crate) fn folders(&self, store: &SessionStore) -> BTreeSet<String> {
        let mut folders = self.lock().folders.clone();
        for view in store.views() {
            let mut found = BTreeSet::new();
            if let Some(session) = view.state.get("session") {
                note_folders(&mut found, session);
            }
            folders.insert(view.session_id, found);
        }
        folders.into_values().flatten().collect()
    }
}

fn note_folders(into: &mut BTreeSet<String>, session: &Value) {
    if let Some(cwd) = session.get("cwd").and_then(Value::as_str) {
        into.insert(cwd.to_owned());
    }
    if let Some(roots) = session.get("workspaceRoots").and_then(Value::as_array) {
        into.extend(roots.iter().filter_map(Value::as_str).map(str::to_owned));
    }
}

/// Drops control characters and clips to `TITLE_MAX_CHARS` (with an ellipsis).
fn clean(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    shorten(&flat)
}

fn shorten(text: &str) -> String {
    if text.chars().count() > TITLE_MAX_CHARS {
        let head: String = text.chars().take(TITLE_MAX_CHARS).collect();
        format!("{head}...")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deleted_sessions_cannot_be_restored_by_a_late_listing() {
        let names = Names::default();
        let store = SessionStore::new();
        let row = json!({"id":"gone", "title":"Gone", "cwd":"/work", "workspaceRoots":["/extra"]});
        names.note_listing(std::slice::from_ref(&row));
        names.note_listing(&[json!({"id":"kept", "cwd":"/work"})]);
        names.forget("gone");
        names.note_listing(&[row]);
        assert_eq!(names.cwd_of(&store, "gone"), None);
        assert_eq!(names.title_of(&store, "gone"), None);
        assert!(!names.is_session_folder(&store, "/extra"));
        assert!(names.is_session_folder(&store, "/work"));
        names.forget("kept");
        assert!(names.folders(&store).is_empty());
    }

    #[test]
    fn current_roots_replace_old_listing_and_loaded_state_wins() {
        let names = Names::default();
        let store = SessionStore::new();
        names.note_listing(&[json!({"id":"s", "cwd":"/old", "workspaceRoots":["/old-extra"]})]);
        names.note_listing(&[json!({"id":"s", "cwd":"/listed"})]);
        assert_eq!(names.folders(&store), BTreeSet::from(["/listed".into()]));
        let _ = store.load(json!({"session":{"id":"s", "cwd":"/live", "workspaceRoots":["/live-extra"]}, "eventId":0}), vw_session::LoadKind::New).unwrap();
        assert_eq!(
            names.folders(&store),
            BTreeSet::from(["/live".into(), "/live-extra".into()])
        );
    }

    #[test]
    fn display_names_collapse_whitespace_while_confirmation_preserves_controls() {
        let names = Names::default();
        let store = SessionStore::new();
        let title = "My \"chat\"\n\u{202E} next";
        names.note_listing(&[
            json!({"id":"controls", "title":title}),
            json!({"id":"spacing", "title":format!("{}Forecast", " ".repeat(100))}),
        ]);
        assert_eq!(
            names.display_title(&store, "controls"),
            "My \"chat\" \u{202E} next"
        );
        assert_eq!(names.confirmation_title(&store, "controls"), title);
        assert_eq!(names.display_title(&store, "spacing"), "Forecast");
    }

    #[test]
    fn visible_names_use_titles_then_previews_then_untitled() {
        let names = Names::default();
        let store = SessionStore::new();
        names.note_listing(&[
            json!({"id":"with-title", "title":"Forecast", "preview":"Check the weather"}),
            json!({"id":"with-preview", "title":" ", "preview":"Check the weather"}),
            json!({"id":"without-name", "title":"", "preview":""}),
        ]);
        assert_eq!(names.display_title(&store, "with-title"), "Forecast");
        assert_eq!(
            names.display_title(&store, "with-preview"),
            "Check the weather"
        );
        assert_eq!(names.display_title(&store, "without-name"), "Untitled chat");
        assert_eq!(names.display_title(&store, "not-listed"), "Untitled chat");
    }
}
