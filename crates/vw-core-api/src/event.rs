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
    /// A permission prompt opened with a shared deadline.
    AskOpened {
        /// Value-free pending metadata.
        ask: vw_ask::Ask,
    },
    /// A permission prompt has one terminal outcome.
    AskClosed {
        /// In-memory value-free record.
        record: vw_ask::AskRecord,
    },
    /// The session grants Vibe Work keeps for Vibe's approvals changed (kept, removed, or ended with a session).
    AskGrants {
        /// Every kept grant, oldest first.
        grants: Vec<vw_ask::SessionGrant>,
        /// What "Allow for this session" would keep on each open card that offers it (its covers line).
        offers: Vec<vw_ask::GrantOffer>,
    },
    /// A Chrome Control card opened, or a call the user set to Never was refused (the session's transcript shows it).
    ChromeAsk {
        /// The card's entry; its outcome is the ask record with the same id.
        entry: crate::ChromeAskView,
    },
    /// Settings › Browser › Your Chrome changed (rows saved, set up, removed).
    ChromeControl {
        /// The view after the change.
        view: crate::ChromeControlView,
    },
    /// One guard stopped an agent action; the summary is the step's own redacted refusal text.
    GuardRefused {
        /// Owning chat (a subagent belongs to its parent).
        session_id: String,
        /// Display title at the decision.
        title: String,
        /// Guard class.
        guard: crate::GuardKind,
        /// Short plain refusal, with no command input values.
        what: String,
        /// History entry or tool-call id; absent outside a chat step.
        step_id: Option<String>,
        /// Unix milliseconds (within JavaScript integer precision).
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        at: u64,
        /// Run-local identity, retained during subscription replay.
        refusal_id: String,
    },
    /// A live proposal changed; snapshots retain its read-only review.
    ProtectedChange {
        /// Review without complete file contents.
        proposal: crate::ProtectedChangeView,
    },
    /// A one-time reading for the window's narration queue; never changes a voice setting.
    ReadAloud {
        /// First-speech receipt, answered only by the listening window.
        request_id: String,
        /// The credential's session.
        session_id: String,
        /// Its chat title, also when a different chat is shown.
        title: String,
        /// The final reply or requested part, at most 10,000 characters before speech cleanup.
        text: String,
        /// End with a spoken notice that the rest is on screen.
        truncated: bool,
    },
    /// The agent's `panel_read`: the window answers with what the session's side panel shows (`files`
    /// `panelAnswer`), and shows a short "Vibe is reading this editor" line meanwhile.
    PanelRead {
        /// Answered once, by the first window that has the panel.
        request_id: String,
        /// The credential's session: only its own panel is read.
        session_id: String,
    },
    /// GitHub reader opt-in or account status changed.
    GithubReads {
        /// Current account and opt-in, never credentials.
        view: crate::GithubReadsView,
    },
    /// Idle sleep request changed.
    KeepAwake {
        /// Current app-wide view.
        view: crate::KeepAwakeView,
    },
    /// Scan status and one retained security finding per chat; no values, paths or byte ranges.
    SecretScan {
        /// Complete value-free scanner view.
        view: crate::SecretScanView,
    },
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
    /// A project tree changed; clients request a fresh bounded snapshot.
    GitBrowserChanged {
        /// Saved project id.
        project_id: String,
    },
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
    /// Messages that waited in a session's turn queue when its turn was stopped or failed, taken out of Vibe's queue
    /// so they are never sent (`queue_return`); the window puts their text and attachments back into the composer.
    QueueReturned {
        /// The session.
        session_id: String,
        /// Vibe's queued items as they were (`id`, `createdAt`, `entries`), oldest first.
        items: Vec<Value>,
    },
    /// Another process (the Vibe CLI, another app) holds a session a window shows, or let it go (Vibe
    /// 2.26.1 lease holder): while held the session is read-only here.
    SessionHolder {
        /// The session.
        session_id: String,
        /// The holding process's pid; `null`: free (or held by Vibe Work itself).
        holder_pid: Option<u32>,
        /// `window` for an app lease, `terminal` for Vibe's native foreign holder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        holder_kind: Option<String>,
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
        /// What that answer said, or none after changing the active key.
        limits: Option<RateLimitsView>,
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
    /// A person right-clicked the page shown in the session's Project tab: the window opens its page menu at
    /// (`x`, `y`) of the page area (CSS pixels). Only the point and whether the click was on selected text; no page
    /// text.
    PageMenu {
        /// The session.
        session_id: String,
        /// Left of the click in the page area.
        x: f64,
        /// Top of the click in the page area.
        y: f64,
        /// The click was on selected text.
        selected: bool,
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
    /// The user switched a session, or the default profile for new sessions, from a lower mode to Full
    /// access (`auto-approve`), or to a profile Vibe reports as bypassing tool permissions (a custom
    /// profile TOML can redefine a profile name such as `ask`). Not raised when a session is reopened in
    /// its mode, nor for sessions that start under the default.
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
    /// A saved model/thinking pick could not be restored; the chat stays open with an explained fallback.
    ModelChoiceNotRestored,
    /// A temporary chat could not be deleted; its retry record remains.
    TemporaryCleanupFailed,
    /// A temporary chat left by a previous run has now been deleted.
    TemporaryCleanupRecovered,
    /// A temporary Mistral source upload could not be deleted after retries.
    RemoteFileCleanupFailed,
    /// A Libraries listing returned completed pages with an explicit incompleteness warning.
    LibraryListPartial,
    /// Vibe refused to re-pin a chat after the installed plugins changed.
    PluginReloadFailed,
    /// Saved network settings could not be authenticated through the key service's vault.
    NetworkSettingsRejected,
    /// The managed config disables MCP, including every Vibe Work tool.
    ToolsUnavailable,
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
        /// The core's resolved access mode.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        access_mode: Option<String>,
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
            Self::Notice { session_id, .. } => session_id.as_deref(),
        }
    }

    /// True when the change concerns `id`.
    #[must_use]
    pub fn concerns(&self, id: &str) -> bool {
        self.session_id() == Some(id)
    }

    /// Mirrors a store change; `None` for `Replaced` (delivered as a snapshot instead).
    #[must_use]
    pub fn from_change(change: Change) -> Option<Self> {
        Some(match change {
            Change::Replaced { .. } => return None,
            Change::Removed { session_id } => Self::Removed { session_id },
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
                access_mode: None,
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
        let mut event = match delivery {
            Delivery::Snapshot(view) => Some(Self::Snapshot {
                snapshot: (*view).into(),
            }),
            Delivery::Change(change) => {
                SessionChange::from_change(change).map(|change| Self::Change { change })
            }
        }?;
        crate::provider_errors::event(&mut event);
        Some(event)
    }

    /// True when a client following only session `id` should receive this event. Events that
    /// belong to no session (server state, flags, refusals) go to every client.
    #[must_use]
    pub fn concerns(&self, id: &str) -> bool {
        match self {
            Self::AskOpened { ask } => ask.session.as_deref().is_none_or(|s| s == id),
            Self::AskClosed { record } => record.ask.session.as_deref().is_none_or(|s| s == id),
            Self::ProtectedChange { proposal } => proposal.session_id == id,
            Self::ChromeAsk { entry } => entry.session_id == id,
            Self::Snapshot { snapshot } => snapshot.session_id == id,
            Self::Change { change } => change.concerns(id),
            Self::GuardRefused { session_id, .. }
            | Self::ReadAloud { session_id, .. }
            | Self::PanelRead { session_id, .. }
            | Self::CallbackOpened { session_id, .. }
            | Self::CallbackAnswered { session_id, .. }
            | Self::SessionStuck { session_id, .. }
            | Self::SessionHolder { session_id, .. }
            | Self::QueueReturned { session_id, .. }
            | Self::LineCommentsChanged { session_id }
            | Self::ChangesUpdated { session_id, .. }
            | Self::PreviewShowRequested { session_id }
            | Self::PageMenu { session_id, .. } => session_id == id,
            Self::Alert {
                session_id: Some(s),
                ..
            } => s == id,
            Self::Terminal { event } => event.session_id() == id,
            Self::PreviewChanged { session_id }
            | Self::BrowserChanged { session_id }
            | Self::PagesChanged { session_id }
            | Self::FilesChanged { session_id } => session_id.as_deref().is_none_or(|s| s == id),
            Self::GitBrowserChanged { .. }
            | Self::Lagged {}
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
            | Self::MarketInstall { .. }
            | Self::KeepAwake { .. }
            | Self::GithubReads { .. }
            | Self::AskGrants { .. }
            | Self::ChromeControl { .. }
            | Self::SecretScan { .. } => true,
        }
    }
}
