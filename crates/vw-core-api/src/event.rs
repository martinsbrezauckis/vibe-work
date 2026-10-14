//! What subscribers of the core receive: session snapshots and ordered changes from the store,
//! plus the core's own events (server state, callbacks opened, refusals, alerts, Debug API flag).
//!
//! Depends on: vw-session (`Change`, `Delivery`), `view`, `command::Origin`. Called by:
//! `subscription` (builds the stream), `core` and `pump` (emit), vw-debug-api (SSE), the UI.
//! Invariants:
//! - Per session, changes arrive in the order the store applied them, after that session's
//!   snapshot (fan-out). A `lagged` event means the client fell behind: fresh snapshots of every
//!   session follow, and changes in between are covered by them.
//! - Core events are ordered among themselves; their order relative to store changes is not
//!   defined (two sources).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vw_session::{Change, Delivery};

use crate::changes_view::ChangesMode;
use crate::command::Origin;
use crate::documents_view::{DocumentRefusal, DocumentSource, DocumentView};
use crate::keys::RateLimitsView;
use crate::speech_view::DictationUpdate;
use crate::terminal_view::TerminalEventView;
use crate::view::{DebugApiFlag, ServerStatus, SessionSnapshot};

/// A subagent's open card as the parent's view needs it (see `CoreEvent::CallbackOpened`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SubagentCard {
    /// The session whose subagent asks (the channel the card arrived on).
    pub parent_session_id: String,
    /// Vibe's callback entry (`callback/call` params' `callback`: id, title, detail, choices, state).
    pub entry: Value,
}

/// One item of a core subscription.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CoreEvent {
    /// Full state of one session; replaces the client's copy.
    Snapshot {
        /// The session.
        snapshot: SessionSnapshot,
    },
    /// An incremental change to apply after the snapshot it follows.
    Change {
        /// The change.
        change: SessionChange,
    },
    /// The client fell behind and missed items; fresh snapshots follow.
    Lagged {},
    /// The supervisor's state changed.
    Server {
        /// New state.
        status: ServerStatus,
    },
    /// A session copy was started or removed; refresh the worktree count.
    WorktreesChanged {},
    /// The server asked for an answer (`callback/call`); answer with `answerCallback`.
    CallbackOpened {
        /// Session.
        session_id: String,
        /// Callback id.
        callback_id: String,
        /// `approval`, `user_input`, `connector_auth` or a newer kind.
        kind: String,
        /// Set only for a subagent's card (its child session has no channel; the card came on the
        /// parent's channel and the child's own state is not loaded): the parent session and Vibe's callback entry,
        /// so the parent's view can show the card. Answer it with `answerCallback` naming the CHILD session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        subagent: Option<SubagentCard>,
    },
    /// A callback was answered through `answerCallback` and Vibe accepted the answer: who sent
    /// it, so the transcript's audit row can say "via the Debug API" instead of "outside this window".
    /// Answers Vibe settles itself (config, smart approve) carry Vibe's own `approvalSource` instead.
    CallbackAnswered {
        /// Session.
        session_id: String,
        /// Callback id.
        callback_id: String,
        /// The client that answered.
        origin: Origin,
    },
    /// A command was refused (gate refusal or invalid command; `ErrorCode::is_refusal`: a `notFound` answer is
    /// not one). Never carries params values.
    Refused {
        /// Who sent it.
        origin: Origin,
        /// The command (`call`, `startTurn`, ...).
        command: String,
        /// The JSON-RPC method involved, when there is one.
        method: Option<String>,
        /// Stable error code (see `ErrorCode`).
        code: String,
        /// Why, without params values.
        reason: String,
    },
    /// Something the user must see at once.
    Alert {
        /// Session concerned, when there is one.
        session_id: Option<String>,
        /// What happened.
        kind: AlertKind,
        /// Text for the UI.
        message: String,
    },
    /// A session became stuck (Vibe could not reopen it after a restart: its commands are refused until
    /// `retryReopen` succeeds or the user continues in a new session) or stopped being stuck.
    SessionStuck {
        /// Session.
        session_id: String,
        /// True while stuck.
        stuck: bool,
        /// The session `continueInNewSession` started for it, when there is one.
        continued_in: Option<String>,
    },
    /// Another process (the Vibe CLI, another app) holds a session a window shows, or let it go (Vibe
    /// 2.26.1 lease holder): while held the session is read-only here.
    SessionHolder {
        /// The session.
        session_id: String,
        /// The holding process's pid; `null`: free (or held by Vibe Work itself).
        holder_pid: Option<u32>,
    },
    /// The Debug API was switched on or off.
    DebugApi {
        /// New flag.
        flag: DebugApiFlag,
    },
    /// A model request waits for the Mistral key: `unlock` (the vault is locked) or
    /// `setup` (no key yet). The UI asks; the turn waits (bounded) instead of failing silently.
    KeyNeeded {
        /// `unlock` or `setup`.
        reason: String,
    },
    /// Vault or key status changed (unlock, lock, key stored or removed): read `keys/readStatus`.
    KeyStatusChanged {},
    /// The Vibe install changed: phase, progress, the offer, existing installs.
    VibeInstall {
        /// The whole install view.
        view: vw_vibe_install::InstallView,
    },
    /// A Mistral answer reported the per-minute rate-limit headroom. The latest snapshot is
    /// also in `readStatus` (`rateLimits`).
    RateLimits {
        /// What that answer said.
        limits: RateLimitsView,
    },
    /// A terminal was opened, exited, restarted or closed.
    Terminal {
        /// What happened.
        event: TerminalEventView,
    },
    /// Files arrived through the native picker or an OS drop: the UI puts them into the
    /// selected session's composer as chips.
    DocumentsAdded {
        /// Picker or drop.
        via: DocumentSource,
        /// Documents added.
        added: Vec<DocumentView>,
        /// Files not added, with the reason.
        refused: Vec<DocumentRefusal>,
    },
    /// A document's state changed (reading started, text ready, reading failed).
    DocumentChanged {
        /// Its facts now.
        document: DocumentView,
    },
    /// A session's changes differ from the last refresh: files edited (by the agent, a command
    /// or the user), kept or reverted. Read them with the `changes` command.
    ChangesUpdated {
        /// Session.
        session_id: String,
        /// Which view the session has now.
        mode: ChangesMode,
        /// Changed files (git mode) or changed file entries (effects mode).
        files: u32,
        /// The new fingerprint.
        fingerprint: String,
    },
    /// A session's preview changed (page loaded, title, refused navigation, download, open, close): read it with
    /// `preview` / `list`. `None`: several changed (the forwarder lagged), re-read every shown preview.
    PreviewChanged {
        /// The session.
        session_id: Option<String>,
    },
    /// An agent requested a visible app preview at a device width. The UI may show the Browser tab only when
    /// this session is selected; no window focus or session switch is implied.
    PreviewShowRequested {
        /// The requesting session.
        session_id: String,
    },
    /// A session's agent browser changed (the agent started or ended an action, Stop, Resume, the
    /// action log): read it with `browser` / `state`. `None`: several changed.
    BrowserChanged {
        /// The session.
        session_id: Option<String>,
    },
    /// A session's Project Pages changed (the viewer loaded or refused a page, a comment was added or removed):
    /// re-read with `pages`. `None`: several changed, re-read what is shown.
    PagesChanged {
        /// The session.
        session_id: Option<String>,
    },
    /// The user's project groups changed (a project created, renamed, deleted, placed, a session filed or moved;
    /// Re-read with `projects list`.
    ProjectsChanged {},
    /// Project secrets, their grants or the use log changed: re-read with `secrets list`. Never a value.
    SecretsChanged {},
    /// A session's File tab viewer changed (an HTML page or PDF loaded, a link was refused): re-read
    /// with `files viewer`. `None`: several changed.
    FilesChanged {
        /// The session.
        session_id: Option<String>,
    },
    /// A session's diff comments changed (added, removed, marked sent): re-read with `lineComments`.
    LineCommentsChanged {
        /// The session.
        session_id: String,
    },
    /// A running dictation's live text, or why live text or the stream ended. Not tied to a
    /// session: the composer that started the stream knows its id.
    Dictation {
        /// The stream (`speech` `start`).
        stream_id: String,
        /// What happened.
        update: DictationUpdate,
    },
    /// A server sign-in started with `connectors` `signInServer` ended. The page address came
    /// earlier as the `mcp_catalog/authUrl` notice; no token is ever part of an event.
    ConnectorSignIn {
        /// The server.
        name: String,
        /// `signedIn`, `failed` or `cancelled`.
        outcome: String,
        /// Vibe's reason when it failed (plain text), else `null`.
        message: Option<String>,
    },
    /// A marketplace install moved on: a phase (`download`, `unpack`, `setup`, `finish`) or its outcome
    /// (`installed`, `failed`, `cancelled`).
    MarketInstall {
        /// Where it is.
        progress: crate::market_view::InstallProgress,
    },
}

/// Kinds of alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum AlertKind {
    /// After a profile switch that needed no confirmation, Vibe reports that tool permissions are
    /// bypassed (a custom profile TOML can redefine a profile name such as `ask`).
    BypassToolPermissions,
    /// A session kept asking for re-reads (its state would not settle); the core stopped re-reading
    /// it for a while instead of hammering Vibe. The session may show stale data until it settles.
    ResyncPaused,
    /// A session that ran in a profile needing a confirmation (`auto-approve`) was reopened by the
    /// core (a new channel, a restart) in `ask` instead: the core cannot confirm for the user.
    AccessModeLowered,
    /// The core could not put a reopened session back in its access mode (the switch failed or Vibe
    /// reported another mode): it closed the session's channel and refuses its commands until a
    /// later open restores it.
    AccessModeUnverified,
    /// A turn that was running when Vibe stopped came back from the restart without having finished
    /// (Vibe reports it `completed`); the core marked it `interrupted`.
    TurnsInterrupted,
    /// A session that had a channel could not be reopened after a server restart (the resume failed or
    /// timed out); its running turns were marked `interrupted`. The next command tries again.
    ReopenFailed,
}

/// Serializable mirror of `vw_session::Change` (minus `Replaced`, which subscribers receive as a
/// snapshot). Field meanings are documented on `Change`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionChange {
    /// The session was dropped from the store.
    Removed {
        /// Session.
        session_id: String,
    },
    /// The conversation moved to a new session id; a snapshot of the new one follows.
    HandedOff {
        /// Id it had.
        old_session_id: String,
        /// Id it has now.
        new_session_id: String,
    },
    /// JSON Patch applied to `state.session`.
    SessionPatched {
        /// Session.
        session_id: String,
        /// Patch operations.
        patch: Vec<Value>,
    },
    /// A history entry was appended or replaced by id.
    EntryUpserted {
        /// Session.
        session_id: String,
        /// The entry as stored.
        entry: Value,
        /// An entry with this id existed and was overwritten.
        replaced: bool,
    },
    /// JSON Patch applied to one history entry.
    EntryPatched {
        /// Session.
        session_id: String,
        /// Entry id.
        entry_id: String,
        /// Patch operations.
        patch: Vec<Value>,
    },
    /// A turn was added or replaced by id.
    TurnUpserted {
        /// Session.
        session_id: String,
        /// The turn.
        turn: Value,
    },
    /// The turn queue was replaced.
    QueueReplaced {
        /// Session.
        session_id: String,
        /// The new queue.
        queue: Value,
    },
    /// `session.tokenUsage` was set.
    TokenUsageSet {
        /// Session.
        session_id: String,
        /// `{inputTokens, outputTokens, totalTokens}`.
        token_usage: Value,
        /// `{contextTokens, contextWindow}` when the stats event carried both.
        context_usage: Option<Value>,
    },
    /// A child session row was added or replaced.
    ChildUpserted {
        /// Session.
        session_id: String,
        /// The child row.
        child: Value,
    },
    /// The set of open callbacks changed.
    CallbacksChanged {
        /// Session.
        session_id: String,
        /// Open callback ids.
        open_callback_ids: Vec<String>,
    },
    /// The session's latest runtime snapshot.
    RuntimeUpdated {
        /// Session.
        session_id: String,
        /// `RuntimeSnapshot`.
        runtime: Value,
    },
    /// A transient notice (`turn/retrying`, `warning`, `error`, auth required).
    Notice {
        /// Session, when named.
        session_id: Option<String>,
        /// Notification method.
        method: String,
        /// Notification params.
        params: Value,
    },
}

impl SessionChange {
    /// The session the change belongs to (`None` for a notice that names none). A hand-off
    /// belongs to both ids; this returns the new one.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Removed { session_id }
            | Self::SessionPatched { session_id, .. }
            | Self::EntryUpserted { session_id, .. }
            | Self::EntryPatched { session_id, .. }
            | Self::TurnUpserted { session_id, .. }
            | Self::QueueReplaced { session_id, .. }
            | Self::TokenUsageSet { session_id, .. }
            | Self::ChildUpserted { session_id, .. }
            | Self::CallbacksChanged { session_id, .. }
            | Self::RuntimeUpdated { session_id, .. } => Some(session_id),
            Self::HandedOff { new_session_id, .. } => Some(new_session_id),
            Self::Notice { session_id, .. } => session_id.as_deref(),
        }
    }

    /// True when the change concerns `id` (hand-offs match either side).
    #[must_use]
    pub fn concerns(&self, id: &str) -> bool {
        match self {
            Self::HandedOff {
                old_session_id,
                new_session_id,
            } => old_session_id == id || new_session_id == id,
            other => other.session_id() == Some(id),
        }
    }

    /// Mirrors a store change; `None` for `Replaced` (delivered as a snapshot instead).
    #[must_use]
    pub fn from_change(change: Change) -> Option<Self> {
        Some(match change {
            Change::Replaced { .. } => return None,
            Change::Removed { session_id } => Self::Removed { session_id },
            Change::HandedOff {
                old_session_id,
                new_session_id,
            } => Self::HandedOff {
                old_session_id,
                new_session_id,
            },
            Change::SessionPatched { session_id, patch } => {
                Self::SessionPatched { session_id, patch }
            }
            Change::EntryUpserted {
                session_id,
                entry,
                replaced,
            } => Self::EntryUpserted {
                session_id,
                entry,
                replaced,
            },
            Change::EntryPatched {
                session_id,
                entry_id,
                patch,
            } => Self::EntryPatched {
                session_id,
                entry_id,
                patch,
            },
            Change::TurnUpserted { session_id, turn } => Self::TurnUpserted { session_id, turn },
            Change::QueueReplaced { session_id, queue } => {
                Self::QueueReplaced { session_id, queue }
            }
            Change::TokenUsageSet {
                session_id,
                token_usage,
                context_usage,
            } => Self::TokenUsageSet {
                session_id,
                token_usage,
                context_usage,
            },
            Change::ChildUpserted { session_id, child } => {
                Self::ChildUpserted { session_id, child }
            }
            Change::CallbacksChanged {
                session_id,
                open_callback_ids,
            } => Self::CallbacksChanged {
                session_id,
                open_callback_ids,
            },
            Change::RuntimeUpdated {
                session_id,
                runtime,
            } => Self::RuntimeUpdated {
                session_id,
                runtime,
            },
            Change::Notice {
                session_id,
                method,
                params,
            } => Self::Notice {
                session_id,
                method,
                params,
            },
        })
    }
}

impl CoreEvent {
    /// Converts one store delivery; `None` for an item that has no mirror.
    #[must_use]
    pub fn from_delivery(delivery: Delivery) -> Option<Self> {
        match delivery {
            Delivery::Snapshot(view) => Some(Self::Snapshot {
                snapshot: (*view).into(),
            }),
            Delivery::Change(change) => {
                SessionChange::from_change(change).map(|change| Self::Change { change })
            }
        }
    }

    /// True when a client following only session `id` should receive this event. Events that
    /// belong to no session (server state, flags, refusals) go to every client.
    #[must_use]
    pub fn concerns(&self, id: &str) -> bool {
        match self {
            Self::Snapshot { snapshot } => snapshot.session_id == id,
            Self::Change { change } => change.concerns(id),
            Self::CallbackOpened { session_id, .. }
            | Self::CallbackAnswered { session_id, .. }
            | Self::SessionStuck { session_id, .. }
            | Self::SessionHolder { session_id, .. }
            | Self::LineCommentsChanged { session_id }
            | Self::ChangesUpdated { session_id, .. }
            | Self::PreviewShowRequested { session_id } => session_id == id,
            Self::Alert {
                session_id: Some(s),
                ..
            } => s == id,
            Self::Terminal { event } => event.session_id() == id,
            Self::PreviewChanged { session_id }
            | Self::BrowserChanged { session_id }
            | Self::PagesChanged { session_id }
            | Self::FilesChanged { session_id } => session_id.as_deref().is_none_or(|s| s == id),
            Self::Lagged {}
            | Self::Server { .. }
            | Self::WorktreesChanged {}
            | Self::Refused { .. }
            | Self::Alert { .. }
            | Self::DebugApi { .. }
            | Self::KeyNeeded { .. }
            | Self::KeyStatusChanged {}
            | Self::VibeInstall { .. }
            | Self::RateLimits { .. }
            | Self::ProjectsChanged {}
            | Self::SecretsChanged {}
            | Self::DocumentsAdded { .. }
            | Self::DocumentChanged { .. }
            | Self::Dictation { .. }
            | Self::ConnectorSignIn { .. }
            | Self::MarketInstall { .. } => true,
        }
    }
}
