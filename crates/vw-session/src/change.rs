//! Output vocabulary of the store: what changed (for fan-out) and what the caller must do.
//!
//! Responsibility: plain data types shared by the reducer, the store and subscribers. Every
//! variant carries the owning `session_id` so a single subscriber can follow many sessions.
//! Depends on: `serde_json`, `anomaly`. Called by: `reducer`, `store`, `fanout`, the UI bridge.

use serde_json::Value;

use crate::anomaly::AnomalyCounts;

/// One incoming server notification: JSON-RPC `method` and `params`, exactly as on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// Method name, e.g. `history/entryAdded`.
    pub method: String,
    /// Params object, camelCase keys.
    pub params: Value,
}

impl Notification {
    /// Builds a notification from its parts.
    #[must_use]
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }
}

/// A read-only copy of one session's state of record plus store bookkeeping.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    /// Session id (`state.session.id`).
    pub session_id: String,
    /// The full `PublicSessionState` as wire JSON (camelCase), including fields we do not model.
    pub state: Value,
    /// Highest event id applied (the watermark).
    pub last_event_id: u64,
    /// True for a session we created that the server's `session/list` does not show yet.
    pub local_only: bool,
    /// True while a gap or failed patch is waiting for the caller to reload from `session/read`.
    pub resync_pending: bool,
    /// Turn ids that this store marked `interrupted` itself after a resume.
    pub interrupted_turns: Vec<String>,
    /// Anomaly counters.
    pub anomalies: AnomalyCounts,
}

/// A state change, in the order it happened. Subscribers apply these to their copy of the state.
/// Every variant carries `session_id`, the session it belongs to (except where noted).
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// The whole state was replaced (load, `session/snapshot`, handoff target). Subscribers get a
    /// [`crate::Delivery::Snapshot`] instead of this variant; it appears only in [`Applied`].
    Replaced {
        /// Session whose state was replaced.
        session_id: String,
    },
    /// The session was dropped from the store.
    Removed {
        /// Session that was removed.
        session_id: String,
    },
    /// `session/compacted` / `session/contextCleared`: the conversation moved to a new session
    /// id. A snapshot of the new session follows.
    HandedOff {
        /// Id the conversation had.
        old_session_id: String,
        /// Id it has now.
        new_session_id: String,
    },
    /// `session/updated`: this JSON Patch was applied to `state.session`.
    SessionPatched {
        /// Session that changed.
        session_id: String,
        /// The wire patch operations, in order.
        patch: Vec<Value>,
    },
    /// A history entry was appended, or replaced by id when the server re-announced it.
    EntryUpserted {
        /// Session that changed.
        session_id: String,
        /// The full entry as it is now stored.
        entry: Value,
        /// True when an entry with this id already existed and was overwritten.
        replaced: bool,
    },
    /// This JSON Patch was applied to the history entry `entry_id`.
    EntryPatched {
        /// Session that changed.
        session_id: String,
        /// Entry the patch targets.
        entry_id: String,
        /// The wire patch operations, in order.
        patch: Vec<Value>,
    },
    /// A turn was added or replaced by id.
    TurnUpserted {
        /// Session that changed.
        session_id: String,
        /// The full turn as it is now stored.
        turn: Value,
    },
    /// The turn queue was replaced.
    QueueReplaced {
        /// Session that changed.
        session_id: String,
        /// The new `PublicTurnQueue`.
        queue: Value,
    },
    /// `session.tokenUsage` was set from a stats event.
    TokenUsageSet {
        /// Session that changed.
        session_id: String,
        /// `{inputTokens, outputTokens, totalTokens}`.
        token_usage: Value,
        /// `{contextTokens, contextWindow}` when the stats event carried both:
        /// `contextWindow` is Vibe's compaction threshold, so the share is "until compaction".
        /// Stored as `session.contextFill` (Vibe's own `session.contextUsage` is a different thing: the
        /// token usage of the current context).
        context_usage: Option<Value>,
    },
    /// A child session row was added or replaced.
    ChildUpserted {
        /// Session that changed.
        session_id: String,
        /// The `PublicChildSession` row.
        child: Value,
    },
    /// The set of open callbacks (`activeCallbacks`) changed.
    CallbacksChanged {
        /// Session that changed.
        session_id: String,
        /// Callback ids that are open now.
        open_callback_ids: Vec<String>,
    },
    /// `runtime/updated`: the session's latest runtime snapshot (not part of the session state).
    RuntimeUpdated {
        /// Session the runtime belongs to.
        session_id: String,
        /// The `RuntimeSnapshot` object.
        runtime: Value,
    },
    /// A transient notice that is not state: `turn/retrying`, `warning`, `error`, auth required.
    Notice {
        /// Session it concerns, when the params name one.
        session_id: Option<String>,
        /// Notification method.
        method: String,
        /// Notification params, untouched.
        params: Value,
    },
}

/// Something the caller (the transport owner) has to do; the store never calls Vibe itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// State may be stale: call `session/read` for `session_id` and pass the result to `load`.
    NeedsResync {
        /// Session to re-read.
        session_id: String,
        /// Why (gap, patch failure, bad envelope).
        reason: String,
    },
}

/// Result of feeding the store one notification or load.
#[must_use = "actions in the result (resync requests) must be carried out"]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// Changes in order, already sent to subscribers.
    pub changes: Vec<Change>,
    /// Follow-up work for the caller.
    pub actions: Vec<Action>,
}
