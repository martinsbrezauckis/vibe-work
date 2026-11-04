//! Older-history adoption under the store lock. The core supplies Vibe pages; subscribers receive snapshots.
//! Generation/cursor checks and entry/JSON-byte budgets bound adoption; existing live entries always win.
use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::{Change, SessionStore};

/// Background hydration limit. Live events are never dropped to enforce this bound.
pub const HISTORY_LIMIT: usize = 20_000;
// Compact JSON bytes of the actual projected Value, including keys, escapes and array punctuation.
// 64 MiB bounds retained background history independently of entry size, without shortening entries.
const HISTORY_BYTE_LIMIT: usize = 64 * 1024 * 1024;
static GENERATION: AtomicU64 = AtomicU64::new(1);

struct JsonBytes(usize);

impl Write for JsonBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// Count the stored representation without allocating a serialized copy or a second history cache.
fn json_bytes(value: &Value) -> usize {
    let mut bytes = JsonBytes(0);
    serde_json::to_writer(&mut bytes, value).map_or(usize::MAX, |()| bytes.0)
}

pub(crate) fn generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// Per-session paging metadata, separate from the wire state and the live-event reducer's flags.
#[derive(Debug)]
pub(crate) struct HistoryPaging {
    pub(crate) generation: u64,
    pub(crate) version: u64,
    pub(crate) closed: bool,
    pub(crate) truncated: bool,
}

impl Default for HistoryPaging {
    fn default() -> Self {
        let generation = generation();
        Self {
            generation,
            version: generation,
            closed: false,
            truncated: false,
        }
    }
}

impl crate::session::SessionEntry {
    /// A new snapshot invalidates outstanding reads even when its boundary happens to match.
    pub(crate) fn reset_history(&mut self, next: &Value) {
        self.history_paging.generation = generation();
        self.history_paging.version = self.history_paging.generation;
        self.history_paging.closed = false;
        self.history_paging.truncated &= self.state["historyBeforeCursor"]
            == next["historyBeforeCursor"]
            && self.state.pointer("/history/0/id") == next.pointer("/history/0/id");
    }
}

/// A small, versioned request descriptor; never a second copy of history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRequest {
    /// The store revision that owns this page chain.
    pub generation: u64,
    /// Vibe's opaque backward cursor.
    pub cursor: String,
    /// Maximum number of entries still allowed, capped at Vibe's page limit.
    pub limit: usize,
    /// Number of entries already adopted, for detecting a page chain that makes no progress.
    pub loaded: usize,
}

impl SessionStore {
    /// The next backward read, if this session is open, incomplete and below the safety bound.
    /// Reaching the bound publishes the quiet truncation state once.
    #[must_use]
    pub fn history_request(&self, id: &str) -> Option<HistoryRequest> {
        self.with(|inner| {
            let entry = inner.sessions.get_mut(id)?;
            if entry.history_paging.closed || entry.history_paging.truncated {
                return None;
            }
            let cursor = entry.state["historyBeforeCursor"].as_str()?.to_owned();
            let count = entry.state["history"].as_array()?.len();
            if count >= HISTORY_LIMIT || json_bytes(&entry.state["history"]) >= HISTORY_BYTE_LIMIT {
                entry.history_paging.truncated = true;
                entry.history_paging.version = generation();
                let _ = inner.emit_all(vec![Change::Replaced {
                    session_id: id.to_owned(),
                }]);
                return None;
            }
            Some(HistoryRequest {
                generation: entry.history_paging.generation,
                cursor,
                limit: (HISTORY_LIMIT - count).min(500),
                loaded: count,
            })
        })
    }

    /// Invalidates in-flight pages and prevents another loader until the next session adoption.
    pub fn close_history(&self, id: &str) {
        self.with(|inner| {
            if let Some(entry) = inner.sessions.get_mut(id) {
                entry.history_paging.closed = true;
                entry.history_paging.generation = generation();
            }
        });
    }

    /// Prepends one page, consuming its entries without cloning the loaded history. Returns false if stale.
    /// Existing IDs win over overlap, including entries updated while the read was in flight.
    #[must_use]
    pub fn prepend_history(
        &self,
        id: &str,
        request: &HistoryRequest,
        mut items: Vec<Value>,
        next: Option<String>,
    ) -> bool {
        self.with(|inner| {
            let Some(entry) = inner.sessions.get_mut(id) else {
                return false;
            };
            if entry.history_paging.closed
                || entry.history_paging.generation != request.generation
                || entry.state["historyBeforeCursor"].as_str() != Some(request.cursor.as_str())
            {
                return false;
            }
            // Recount under the adoption lock: live appends/patches may have used the budget since request.
            let mut bytes = json_bytes(&entry.state["history"]);
            let Some(history) = entry.state["history"].as_array_mut() else {
                return false;
            };
            let mut ids: HashSet<String> = history
                .iter()
                .filter_map(|v| v["id"].as_str().map(str::to_owned))
                .collect();
            items.retain(|v| v["id"].as_str().is_some_and(|id| ids.insert(id.to_owned())));
            let available = HISTORY_LIMIT.saturating_sub(history.len());
            let mut kept = 0;
            // Pages are oldest first. Keep a contiguous prefix nearest the loaded suffix; never skip a
            // large entry to load farther ones. Count only unique entries after display projection.
            for item in items.iter().rev().take(available) {
                let extra =
                    json_bytes(item).saturating_add(usize::from(!history.is_empty() || kept > 0));
                if extra > HISTORY_BYTE_LIMIT.saturating_sub(bytes) {
                    break;
                }
                bytes = bytes.saturating_add(extra);
                kept += 1;
            }
            let omitted = items.len() - kept;
            items.drain(..omitted);
            items.append(history);
            *history = items;
            entry.history_paging.truncated = omitted > 0
                || ((history.len() >= HISTORY_LIMIT || bytes >= HISTORY_BYTE_LIMIT)
                    && next.is_some());
            entry.history_paging.version = generation();
            entry.state["historyBeforeCursor"] = if omitted > 0 {
                Value::String(request.cursor.clone())
            } else {
                next.map_or(Value::Null, Value::String)
            };
            let _ = inner.emit_all(vec![Change::Replaced {
                session_id: id.to_owned(),
            }]);
            true
        })
    }
}
