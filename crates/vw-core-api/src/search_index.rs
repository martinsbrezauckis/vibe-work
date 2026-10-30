//! The message index of the search: a core-side index over transcript text, built
//! lazily from history.
//!
//! What it holds: per session, the text of its user and agent messages (`message` entries with role `user` or
//! `assistant`; tool steps, reasoning and output are not searched), with entry id, turn number and time, in memory
//! only. Nothing is written to disk: Vibe already stores the conversations, and a second copy in our app data would
//! need its own deletion rules (temporary sessions, deleted sessions) for little gain at this size.
//!
//! How it fills: the first search starts a background pass ([`Core::refresh_index`]): `session/list` (all pages,
//! archived included), then for every session that is new or whose `updatedAt` moved, the whole stored history
//! (`Core::full_history`, `session/history/list` through the lobby for sessions no channel holds: reading never takes
//! a session from the Vibe CLI). Sessions no longer listed leave the index. A search runs on what is indexed now and
//! says how far the pass is; the UI searches again while it fills. A later search starts a new pass when the last one
//! is older than [`REFRESH_AFTER`], or at once when a loaded session's newest history entry is not in the index yet (a
//! message just sent or answered, which a search right after a turn could otherwise miss). A search
//! scoped to one session reads that session first when it is missing or out of date, so a conversation's own search is
//! complete. Every Vibe call of a pass has a time limit ([`LIST_WAIT`], [`READ_WAIT`]).
//!
//! Depends on: `search_text` (matching), `session_export` (`full_history`), the gate (`session/list`). Called by:
//! `search` (the `search` command).
//! Invariants: at most one pass runs at a time; no lock is held across an await; a failed read of one session keeps
//! its previous text and is retried by the next pass; text per message is capped at [`MAX_TEXT`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tracing::warn;

use crate::core::{Core, Shared};
use crate::error::CoreError;
use crate::search_text::{Query, find_kept, snippet};
use crate::search_view::{IndexStatus, MessageHit};

/// A pass older than this is repeated by the next search.
const REFRESH_AFTER: Duration = Duration::from_secs(15);
/// Most characters of one message kept for searching.
const MAX_TEXT: usize = 100_000;
/// Sessions per `session/list` page.
const LIST_PAGE: u64 = 200;
/// Most listing pages read (10,000 sessions).
const LIST_PAGES: usize = 50;
/// Hits counted at most.
const COUNT_CAP: u32 = 1000;
/// Longest one `session/list` page may take during a pass.
const LIST_WAIT: Duration = Duration::from_secs(15);
/// Longest one session's history may take to read during a pass (a session Vibe does not answer for is skipped and
/// tried again by the next pass).
const READ_WAIT: Duration = Duration::from_secs(20);

/// One searchable message.
#[derive(Debug, Clone)]
struct Message {
    entry_id: String,
    role: String,
    turn: u32,
    at: Option<u64>,
    text: String,
    /// The lowercase copy of an ASCII `text` (same offsets), kept so a search lowercases nothing.
    lower: Option<String>,
}

/// One session's messages.
#[derive(Debug, Clone, Default)]
struct Doc {
    updated_at: Option<i64>,
    /// The id of the history's last entry when it was read (a loaded session whose last entry differs has news).
    last_entry: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
    messages: Vec<Message>,
}

/// A session as the listing showed it.
#[derive(Debug, Clone)]
struct Listed {
    updated_at: Option<i64>,
    title: Option<String>,
    cwd: Option<String>,
}

#[derive(Default)]
struct Inner {
    docs: BTreeMap<String, Doc>,
    listed: BTreeMap<String, Listed>,
    filling: bool,
    last_pass: Option<Instant>,
    /// Loaded sessions with news the next pass reads again whatever their listing says.
    stale: BTreeSet<String>,
    error: Option<String>,
}

/// The index (lives in `Shared`).
#[derive(Default)]
pub(crate) struct MessageIndex {
    inner: Mutex<Inner>,
}

impl MessageIndex {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How far the index is.
    pub(crate) fn status(&self) -> IndexStatus {
        let inner = self.lock();
        IndexStatus {
            indexed: u32::try_from(inner.docs.len()).unwrap_or(u32::MAX),
            total: u32::try_from(inner.listed.len().max(inner.docs.len())).unwrap_or(u32::MAX),
            filling: inner.filling,
            error: inner.error.clone(),
        }
    }

    /// Claims the next pass when none runs and the last one is old (or none ran). True: the caller runs it.
    fn claim_pass(&self, news: bool) -> bool {
        let mut inner = self.lock();
        let due = news
            || inner
                .last_pass
                .is_none_or(|at| at.elapsed() >= REFRESH_AFTER);
        if inner.filling || !due {
            return false;
        }
        inner.filling = true;
        true
    }

    /// True when `session_id`'s indexed text is older than what the store shows: missing, or its last entry differs from
    /// the store's (`store_last`: the id of the last history entry of the loaded session).
    fn behind(&self, session_id: &str, store_last: Option<&str>) -> bool {
        let inner = self.lock();
        match inner.docs.get(session_id) {
            None => store_last.is_some(),
            Some(doc) => store_last.is_some() && doc.last_entry.as_deref() != store_last,
        }
    }

    fn needs(&self, session_id: &str) -> bool {
        let inner = self.lock();
        if inner.stale.contains(session_id) {
            return true;
        }
        match (inner.docs.get(session_id), inner.listed.get(session_id)) {
            (None, _) => true,
            (Some(doc), Some(listed)) => doc.updated_at != listed.updated_at,
            (Some(_), None) => false,
        }
    }

    fn put(&self, session_id: &str, doc: Doc) {
        let mut inner = self.lock();
        inner.stale.remove(session_id);
        inner.docs.insert(session_id.to_owned(), doc);
    }

    /// Drops a session (it was deleted).
    pub(crate) fn forget(&self, session_id: &str) {
        let mut inner = self.lock();
        inner.docs.remove(session_id);
        inner.listed.remove(session_id);
    }

    /// The hits of `query` (best first), at most `limit`, and how many there are in all (capped).
    fn search(&self, query: &Query, scope: Option<&str>, limit: usize) -> (Vec<MessageHit>, u32) {
        let inner = self.lock();
        let mut found: Vec<(bool, Option<i64>, usize, MessageHit)> = Vec::new();
        let mut total: u32 = 0;
        for (session_id, doc) in &inner.docs {
            if scope.is_some_and(|s| s != session_id) {
                continue;
            }
            for (order, message) in doc.messages.iter().enumerate() {
                let Some((at, phrase)) = find_kept(&message.text, message.lower.as_deref(), query)
                else {
                    continue;
                };
                total = total.saturating_add(1).min(COUNT_CAP);
                found.push((
                    phrase,
                    doc.updated_at,
                    order,
                    MessageHit {
                        session_id: session_id.clone(),
                        title: doc.title.clone(),
                        cwd: doc.cwd.clone(),
                        entry_id: message.entry_id.clone(),
                        role: message.role.clone(),
                        turn: message.turn,
                        snippet: snippet(&message.text, at),
                        at: message.at,
                        phrase,
                    },
                ));
            }
        }
        // Scoped (one conversation): in conversation order, for "2 of 5" stepping. Everywhere: phrase matches first,
        // then newer sessions, then newer messages.
        if scope.is_some() {
            found.sort_by_key(|(_, _, order, _)| *order);
        } else {
            found.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
        }
        let hits = found.into_iter().take(limit).map(|(.., hit)| hit).collect();
        (hits, total)
    }
}

/// The messages of a history, oldest first, with turn numbers.
fn messages_of(entries: &[Value]) -> Vec<Message> {
    let mut turn: u32 = 0;
    let mut out = Vec::new();
    for entry in entries {
        if entry.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let role = entry
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if role == "user" {
            turn += 1;
        }
        if role != "user" && role != "assistant" {
            continue;
        }
        let mut text = String::new();
        for block in entry
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                text.push_str(
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        if text.len() > MAX_TEXT {
            let mut cut = MAX_TEXT;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        out.push(Message {
            entry_id: entry
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            role: role.to_owned(),
            turn: turn.max(1),
            at: entry.get("createdAt").and_then(Value::as_u64),
            lower: text.is_ascii().then(|| text.to_ascii_lowercase()),
            text,
        });
    }
    out
}

fn listed_of(item: &Value) -> Option<(String, Listed)> {
    let id = item.get("id").and_then(Value::as_str)?.to_owned();
    Some((
        id,
        Listed {
            updated_at: item.get("updatedAt").and_then(Value::as_i64),
            title: item.get("title").and_then(Value::as_str).map(str::to_owned),
            cwd: item.get("cwd").and_then(Value::as_str).map(str::to_owned),
        },
    ))
}

/// Every session Vibe lists (archived included).
async fn list_all(shared: &Shared) -> Result<BTreeMap<String, Listed>, CoreError> {
    let mut all = BTreeMap::new();
    let mut cursor: Option<String> = None;
    for _ in 0..LIST_PAGES {
        let mut params = json!({"includeArchived": true, "limit": LIST_PAGE});
        if let Some(at) = &cursor {
            params["cursor"] = json!(at);
        }
        let reply = tokio::time::timeout(LIST_WAIT, shared.gate.call("session/list", params, None))
            .await
            .map_err(|_| {
                CoreError::new(crate::ErrorCode::Timeout, "the session list did not answer")
            })?
            .map_err(|e| CoreError::from_gate("session/list", &e))?;
        let mut result = reply.result;
        // The index and the name cache keep titles with vault-value parts withheld (`title_guard`).
        crate::title_guard::withhold_in(shared, &mut result);
        let items = result
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        shared.names.note_listing(&items);
        all.extend(items.iter().filter_map(listed_of));
        cursor = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Ok(all)
}

impl Core {
    /// Reads one session's messages into the index.
    async fn index_session(
        &self,
        session_id: &str,
        listed: Option<Listed>,
    ) -> Result<(), CoreError> {
        let (entries, _) = tokio::time::timeout(READ_WAIT, self.full_history(session_id))
            .await
            .map_err(|_| {
                CoreError::new(
                    crate::ErrorCode::Timeout,
                    "the session's history did not answer",
                )
            })??;
        let from_store = self.shared.store.view(session_id).map(|v| v.state);
        let pick = |pointer: &str| {
            from_store
                .as_ref()
                .and_then(|s| s.pointer(pointer))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let doc = Doc {
            updated_at: listed.as_ref().and_then(|l| l.updated_at),
            last_entry: entries
                .last()
                .and_then(|e| e.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            title: listed
                .as_ref()
                .and_then(|l| l.title.clone())
                .or_else(|| pick("/session/title")),
            cwd: listed
                .as_ref()
                .and_then(|l| l.cwd.clone())
                .or_else(|| pick("/session/cwd")),
            messages: messages_of(&entries),
        };
        self.shared.search.put(session_id, doc);
        Ok(())
    }

    /// One fill pass (module header). Never fails: an error is kept in the status.
    pub(crate) async fn refresh_index(&self) {
        let started = Instant::now();
        let listed = match list_all(&self.shared).await {
            Ok(listed) => listed,
            Err(error) => {
                warn!(%error, "search: the session list could not be read");
                let mut inner = self.shared.search.lock();
                inner.filling = false;
                inner.last_pass = Some(Instant::now());
                inner.error = Some(error.message);
                return;
            }
        };
        {
            let mut inner = self.shared.search.lock();
            let keep: BTreeSet<&String> = listed.keys().collect();
            inner.docs.retain(|id, _| keep.contains(id));
            inner.listed.clone_from(&listed);
            inner.error = None;
        }
        // Newest sessions first, so the likely hits arrive early.
        let mut order: Vec<(&String, &Listed)> = listed.iter().collect();
        order.sort_by_key(|entry| std::cmp::Reverse(entry.1.updated_at));
        for (id, entry) in order {
            if !self.shared.search.needs(id) {
                continue;
            }
            if let Err(error) = self.index_session(id, Some(entry.clone())).await {
                warn!(session_id = %id, %error, "search: a session's history could not be read");
            }
        }
        let mut inner = self.shared.search.lock();
        inner.filling = false;
        inner.last_pass = Some(Instant::now());
        tracing::info!(
            sessions = inner.listed.len(),
            indexed = inner.docs.len(),
            millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "search: index pass done"
        );
    }

    /// Loaded sessions whose newest history entry is not in the index yet (a message typed or answered since).
    fn sessions_with_news(&self) -> Vec<String> {
        self.shared
            .store
            .views()
            .into_iter()
            .filter(|view| {
                let last = view
                    .state
                    .get("history")
                    .and_then(Value::as_array)
                    .and_then(|h| h.last())
                    .and_then(|e| e.get("id"))
                    .and_then(Value::as_str);
                self.shared.search.behind(&view.session_id, last)
            })
            .map(|view| view.session_id)
            .collect()
    }

    /// Starts a fill pass in the background when one is due: the last is older than [`REFRESH_AFTER`], or a loaded
    /// session has news (module header).
    pub(crate) fn kick_index(&self) {
        let news = self.sessions_with_news();
        {
            let mut inner = self.shared.search.lock();
            for id in &news {
                inner.stale.insert(id.clone());
            }
        }
        if !self.shared.search.claim_pass(!news.is_empty()) {
            return;
        }
        let weak = Arc::downgrade(&self.shared);
        tokio::spawn(async move {
            let Some(shared) = weak.upgrade() else { return };
            Core { shared }.refresh_index().await;
        });
    }

    /// `search messages` (see `SearchRequest::Messages`).
    pub(crate) async fn search_messages(
        &self,
        query: &str,
        session_id: Option<&str>,
        limit: usize,
    ) -> (Vec<MessageHit>, u32, IndexStatus) {
        self.kick_index();
        if let Some(id) = session_id
            && self.shared.search.needs(id)
        {
            let listed = self.shared.search.lock().listed.get(id).cloned();
            if let Err(error) = self.index_session(id, listed).await {
                warn!(session_id = id, %error, "search: the conversation could not be read");
            }
        }
        let Some(query) = Query::new(query) else {
            return (Vec::new(), 0, self.shared.search.status());
        };
        let (hits, total) = self.shared.search.search(&query, session_id, limit);
        (hits, total, self.shared.search.status())
    }
}
